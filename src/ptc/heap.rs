//! QuickJS backing allocator that enforces the heap cap.
//!
//! QuickJS applies `JS_SetMemoryLimit` before it calls the allocator, and a
//! refusal there is invisible to us. The cap lives here instead, so a refused
//! allocation is recorded on [`LimitAlloc::fault`] and the engine's later
//! exception can be classified from that flag.

use std::alloc::{self, Layout};
use std::ffi::c_void;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicUsize, Ordering};

use rquickjs::allocator::Allocator;

const ALLOC_ALIGN: usize = align_of::<u64>();
const HEADER_SIZE: usize = if size_of::<usize>() > ALLOC_ALIGN {
    size_of::<usize>()
} else {
    ALLOC_ALIGN
};

#[derive(Copy, Clone)]
#[repr(transparent)]
struct Header {
    size: usize,
}

fn round_size(size: usize) -> Option<usize> {
    size.checked_add(ALLOC_ALIGN - 1)
        .map(|rounded| rounded & !(ALLOC_ALIGN - 1))
}

pub(crate) struct LimitAlloc {
    used: usize,
    /// `0` means unlimited.
    limit: usize,
    fault: Arc<AtomicBool>,
    /// `JSContext*` and the byte offset of `interrupt_counter`, shared with the
    /// engine so a refused allocation can request the next opcode's interrupt.
    ctx: Arc<AtomicPtr<c_void>>,
    counter_offset: Arc<AtomicUsize>,
}

impl LimitAlloc {
    pub(crate) fn new(
        limit: usize,
        fault: Arc<AtomicBool>,
        ctx: Arc<AtomicPtr<c_void>>,
        counter_offset: Arc<AtomicUsize>,
    ) -> Self {
        Self {
            used: 0,
            limit,
            fault,
            ctx,
            counter_offset,
        }
    }

    fn note_fault(&self) {
        self.fault.store(true, Ordering::Release);
        // The interpreter checks the counter at the top of the next opcode.
        // Setting it to 1 makes that check run before a `catch` body can
        // swallow the allocation failure.
        let ctx = self.ctx.load(Ordering::Relaxed);
        let offset = self.counter_offset.load(Ordering::Relaxed);
        if ctx.is_null() || offset == 0 {
            return;
        }
        unsafe {
            ctx.cast::<u8>().add(offset).cast::<i32>().write(1);
        }
    }

    fn exceeds(&self, extra: usize) -> bool {
        self.limit > 0 && self.used.saturating_add(extra) > self.limit
    }

    fn track(&mut self, size: usize) {
        self.used = self.used.saturating_add(size);
    }

    fn untrack(&mut self, size: usize) {
        self.used = self.used.saturating_sub(size);
    }
}

unsafe impl Allocator for LimitAlloc {
    fn alloc(&mut self, size: usize) -> *mut u8 {
        let Some(size) = round_size(size) else {
            self.note_fault();
            return std::ptr::null_mut();
        };
        if self.exceeds(size) {
            self.note_fault();
            return std::ptr::null_mut();
        }
        let Some(layout) =
            Layout::from_size_align(HEADER_SIZE.saturating_add(size), ALLOC_ALIGN).ok()
        else {
            self.note_fault();
            return std::ptr::null_mut();
        };
        let ptr = unsafe { alloc::alloc(layout) };
        if ptr.is_null() {
            self.note_fault();
            return std::ptr::null_mut();
        }
        unsafe {
            ptr.cast::<Header>().write(Header { size });
        }
        self.track(size);
        unsafe { ptr.add(HEADER_SIZE) }
    }

    fn calloc(&mut self, count: usize, size: usize) -> *mut u8 {
        let Some(total) = count.checked_mul(size) else {
            self.note_fault();
            return std::ptr::null_mut();
        };
        if total == 0 {
            return self.alloc(0);
        }
        let Some(total) = round_size(total) else {
            self.note_fault();
            return std::ptr::null_mut();
        };
        if self.exceeds(total) {
            self.note_fault();
            return std::ptr::null_mut();
        }
        let Some(layout) =
            Layout::from_size_align(HEADER_SIZE.saturating_add(total), ALLOC_ALIGN).ok()
        else {
            self.note_fault();
            return std::ptr::null_mut();
        };
        let ptr = unsafe { alloc::alloc_zeroed(layout) };
        if ptr.is_null() {
            self.note_fault();
            return std::ptr::null_mut();
        }
        unsafe {
            ptr.cast::<Header>().write(Header { size: total });
        }
        self.track(total);
        unsafe { ptr.add(HEADER_SIZE) }
    }

    unsafe fn dealloc(&mut self, ptr: *mut u8) {
        unsafe {
            let ptr = ptr.sub(HEADER_SIZE);
            let size = ptr.cast::<Header>().read().size;
            self.untrack(size);
            let layout = Layout::from_size_align_unchecked(HEADER_SIZE + size, ALLOC_ALIGN);
            alloc::dealloc(ptr, layout);
        }
    }

    unsafe fn realloc(&mut self, ptr: *mut u8, new_size: usize) -> *mut u8 {
        let Some(new_size) = round_size(new_size) else {
            self.note_fault();
            return std::ptr::null_mut();
        };
        unsafe {
            let header = ptr.sub(HEADER_SIZE);
            let old_size = header.cast::<Header>().read().size;
            let next_used = self.used.saturating_sub(old_size).saturating_add(new_size);
            if self.limit > 0 && next_used > self.limit {
                self.note_fault();
                return std::ptr::null_mut();
            }
            let old_layout = Layout::from_size_align_unchecked(HEADER_SIZE + old_size, ALLOC_ALIGN);
            let new_ptr = alloc::realloc(header, old_layout, HEADER_SIZE + new_size);
            if new_ptr.is_null() {
                self.note_fault();
                return std::ptr::null_mut();
            }
            new_ptr.cast::<Header>().write(Header { size: new_size });
            self.used = next_used;
            new_ptr.add(HEADER_SIZE)
        }
    }

    unsafe fn usable_size(ptr: *mut u8) -> usize {
        unsafe { ptr.sub(HEADER_SIZE).cast::<Header>().read().size }
    }
}

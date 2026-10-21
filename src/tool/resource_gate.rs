//! One file-identity and one reader-writer queue for tool effects.
//!
//! The key matches what [`crate::workspace::resolve_agent`] does not fold:
//! `.` is dropped, `..` pops one normal component, separators are rebuilt,
//! and on Windows each component loses trailing dots and spaces and is
//! case-folded with the file-system uppercase table. Reads share a key.
//! Writes exclude readers and other writers. Waiters for one key run FIFO.
//! A call that names several keys takes them in sorted order.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

use tokio_util::sync::CancellationToken;

use crate::tool::write_lock::ResourceKey;
use crate::workspace::ToolPathMode;

/// Reads are shared. Writes are exclusive. An empty value takes no lock.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ToolEffects {
    pub reads: Vec<ResourceKey>,
    pub writes: Vec<ResourceKey>,
}

impl ToolEffects {
    pub fn read(key: ResourceKey) -> Self {
        Self {
            reads: vec![key],
            writes: Vec::new(),
        }
    }

    pub fn write(key: ResourceKey) -> Self {
        Self {
            reads: Vec::new(),
            writes: vec![key],
        }
    }

    pub fn declares(&self) -> bool {
        !self.reads.is_empty() || !self.writes.is_empty()
    }
}

/// Lock identity of one path. This is not a path to open: on Windows it is
/// case-folded, and a missing file keeps the caller's unresolved spelling
/// only after `.` / `..` / separator normalization.
pub fn file_resource_key(path: &Path) -> ResourceKey {
    let mut rebuilt = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => pop_preceding_normal(&mut rebuilt),
            Component::Normal(name) => push_normal(&mut rebuilt, name),
            other => rebuilt.push(other.as_os_str()),
        }
    }
    ResourceKey::File(rebuilt.to_string_lossy().into_owned())
}

/// Key for a file the call named. Resolution failure still produces a key
/// from the raw spelling so two calls that fail the same way share a queue.
pub fn lock_key(workspace: &Path, raw: &str, mode: ToolPathMode) -> ResourceKey {
    let path = crate::workspace::resolve_agent(workspace, raw, mode)
        .unwrap_or_else(|_| PathBuf::from(raw));
    file_resource_key(&path)
}

/// Key for a directory scan. An empty path is the workspace root, which is
/// the root glob and grep actually walk when `path` is omitted.
pub fn scan_key(workspace: &Path, raw: Option<&str>, mode: ToolPathMode) -> ResourceKey {
    match raw.map(str::trim).filter(|path| !path.is_empty()) {
        Some(raw) => lock_key(workspace, raw, mode),
        None => file_resource_key(workspace),
    }
}

fn pop_preceding_normal(path: &mut PathBuf) {
    if matches!(path.components().next_back(), Some(Component::Normal(_))) {
        path.pop();
    }
}

fn push_normal(path: &mut PathBuf, name: &std::ffi::OsStr) {
    let name = normalize_normal(name);
    if name.is_empty() || name == std::ffi::OsStr::new(".") {
        return;
    }
    if name == std::ffi::OsStr::new("..") {
        pop_preceding_normal(path);
        return;
    }
    path.push(name);
}

#[cfg(not(windows))]
fn normalize_normal(name: &std::ffi::OsStr) -> std::ffi::OsString {
    name.to_os_string()
}

/// Strip trailing dots and spaces, then apply the Windows file-system
/// uppercase table. `LCMapStringEx` without `LCMAP_LINGUISTIC_CASING` is that
/// table; it does not open the file.
#[cfg(windows)]
fn normalize_normal(name: &std::ffi::OsStr) -> std::ffi::OsString {
    use std::os::windows::ffi::OsStrExt;
    let mut wide: Vec<u16> = name.encode_wide().collect();
    while wide
        .last()
        .is_some_and(|unit| *unit == u16::from(b' ') || *unit == u16::from(b'.'))
    {
        wide.pop();
    }
    windows_fs_upcase(&wide)
}

#[cfg(windows)]
fn windows_fs_upcase(wide: &[u16]) -> std::ffi::OsString {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::Globalization::{
        LCMAP_UPPERCASE, LCMapStringEx, LOCALE_NAME_INVARIANT,
    };
    if wide.is_empty() {
        return std::ffi::OsString::new();
    }
    let Ok(len) = i32::try_from(wide.len()) else {
        return std::ffi::OsString::from_wide(wide);
    };
    let mut mapped = vec![0u16; wide.len()];
    let wrote = unsafe {
        LCMapStringEx(
            LOCALE_NAME_INVARIANT,
            LCMAP_UPPERCASE,
            wide.as_ptr(),
            len,
            mapped.as_mut_ptr(),
            len,
            std::ptr::null::<windows_sys::Win32::Globalization::NLSVERSIONINFO>(),
            std::ptr::null::<core::ffi::c_void>(),
            0,
        )
    };
    if wrote == len {
        std::ffi::OsString::from_wide(&mapped)
    } else {
        std::ffi::OsString::from_wide(wide)
    }
}

#[derive(Clone)]
struct Waiter {
    exclusive: bool,
    granted: Arc<AtomicBool>,
    notify: Arc<tokio::sync::Notify>,
}

struct Slot {
    readers: u32,
    writing: bool,
    queue: VecDeque<Waiter>,
}

impl Default for Slot {
    fn default() -> Self {
        Self {
            readers: 0,
            writing: false,
            queue: VecDeque::new(),
        }
    }
}

struct Inner {
    slots: Mutex<HashMap<ResourceKey, Slot>>,
}

/// Process-wide effect queue. Same-key waiters run FIFO. Shared readers enter
/// together only when no writer is ahead of them.
#[derive(Clone)]
pub struct ResourceGate {
    inner: Arc<Inner>,
}

struct Claim {
    key: ResourceKey,
    exclusive: bool,
}

/// Held from the moment a call owns its keys until the call returns.
pub struct EffectGuard {
    gate: ResourceGate,
    held: Vec<Claim>,
}

impl Drop for EffectGuard {
    fn drop(&mut self) {
        for claim in self.held.drain(..).rev() {
            self.gate.release(&claim.key, claim.exclusive);
        }
    }
}

impl ResourceGate {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                slots: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// Take every declared key. Writes outrank reads of the same key. Keys
    /// are sorted so two calls cannot lock each other in opposite orders.
    /// Cancellation drops any key this call already holds and does not enter.
    pub async fn acquire(
        &self,
        effects: &ToolEffects,
        cancel: &CancellationToken,
    ) -> Result<EffectGuard, ()> {
        let claims = claims(effects);
        let mut held: Vec<Claim> = Vec::with_capacity(claims.len());
        for claim in claims {
            if self
                .acquire_one(claim.key.clone(), claim.exclusive, cancel)
                .await
                .is_err()
            {
                for claim in held.into_iter().rev() {
                    self.release(&claim.key, claim.exclusive);
                }
                return Err(());
            }
            held.push(claim);
        }
        Ok(EffectGuard {
            gate: self.clone(),
            held,
        })
    }

    fn acquire_one(
        &self,
        key: ResourceKey,
        exclusive: bool,
        cancel: &CancellationToken,
    ) -> impl std::future::Future<Output = Result<(), ()>> + Send {
        let gate = self.clone();
        let cancel = cancel.clone();
        async move {
            if cancel.is_cancelled() {
                return Err(());
            }
            let waiter = Waiter {
                exclusive,
                granted: Arc::new(AtomicBool::new(false)),
                notify: Arc::new(tokio::sync::Notify::new()),
            };
            {
                let mut slots = lock_slots(&gate.inner.slots);
                let slot = slots.entry(key.clone()).or_default();
                slot.queue.push_back(waiter.clone());
                grant(slot);
            }
            loop {
                if cancel.is_cancelled() {
                    gate.leave_queue(&key, &waiter);
                    return Err(());
                }
                if waiter.granted.load(Ordering::SeqCst) {
                    return Ok(());
                }
                let notified = waiter.notify.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                if cancel.is_cancelled() {
                    gate.leave_queue(&key, &waiter);
                    return Err(());
                }
                if waiter.granted.load(Ordering::SeqCst) {
                    return Ok(());
                }
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => {
                        gate.leave_queue(&key, &waiter);
                        return Err(());
                    }
                    _ = &mut notified => {}
                }
            }
        }
    }

    fn leave_queue(&self, key: &ResourceKey, waiter: &Waiter) {
        let mut slots = lock_slots(&self.inner.slots);
        let Some(slot) = slots.get_mut(key) else {
            return;
        };
        if waiter.granted.load(Ordering::SeqCst) {
            release_holder(slot, waiter.exclusive);
        } else {
            slot.queue
                .retain(|queued| !Arc::ptr_eq(&queued.granted, &waiter.granted));
        }
        grant(slot);
        if slot.readers == 0 && !slot.writing && slot.queue.is_empty() {
            slots.remove(key);
        }
    }

    fn release(&self, key: &ResourceKey, exclusive: bool) {
        let mut slots = lock_slots(&self.inner.slots);
        let Some(slot) = slots.get_mut(key) else {
            return;
        };
        release_holder(slot, exclusive);
        grant(slot);
        if slot.readers == 0 && !slot.writing && slot.queue.is_empty() {
            slots.remove(key);
        }
    }

    #[cfg(test)]
    fn queued(&self, key: &ResourceKey) -> usize {
        let slots = lock_slots(&self.inner.slots);
        slots.get(key).map(|slot| slot.queue.len()).unwrap_or(0)
    }
}

impl Default for ResourceGate {
    fn default() -> Self {
        Self::new()
    }
}

fn claims(effects: &ToolEffects) -> Vec<Claim> {
    let mut keys: BTreeMap<ResourceKey, bool> = BTreeMap::new();
    for key in &effects.reads {
        keys.entry(key.clone()).or_insert(false);
    }
    for key in &effects.writes {
        keys.insert(key.clone(), true);
    }
    keys.into_iter()
        .map(|(key, exclusive)| Claim { key, exclusive })
        .collect()
}

fn grant(slot: &mut Slot) {
    while let Some(front) = slot.queue.front() {
        if front.exclusive {
            if slot.readers == 0 && !slot.writing {
                let waiter = slot.queue.pop_front().expect("front");
                slot.writing = true;
                waiter.granted.store(true, Ordering::SeqCst);
                waiter.notify.notify_one();
            }
            return;
        }
        if slot.writing {
            return;
        }
        let waiter = slot.queue.pop_front().expect("front");
        slot.readers = slot.readers.saturating_add(1);
        waiter.granted.store(true, Ordering::SeqCst);
        waiter.notify.notify_one();
    }
}

fn release_holder(slot: &mut Slot, exclusive: bool) {
    if exclusive {
        slot.writing = false;
    } else if slot.readers > 0 {
        slot.readers -= 1;
    }
}

fn lock_slots(
    slots: &Mutex<HashMap<ResourceKey, Slot>>,
) -> std::sync::MutexGuard<'_, HashMap<ResourceKey, Slot>> {
    slots.lock().unwrap_or_else(|error| error.into_inner())
}

static PROCESS_RESOURCE_GATE: LazyLock<ResourceGate> = LazyLock::new(ResourceGate::new);

/// Shared by every `execute_tool_call`, direct and nested.
pub fn process_resource_gate() -> ResourceGate {
    PROCESS_RESOURCE_GATE.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn write_key(name: &str) -> ToolEffects {
        ToolEffects::write(ResourceKey::File(name.into()))
    }

    fn read_key(name: &str) -> ToolEffects {
        ToolEffects::read(ResourceKey::File(name.into()))
    }

    async fn until_queued(gate: &ResourceGate, key: &ResourceKey, want: usize) {
        for _ in 0..50 {
            if gate.queued(key) >= want {
                return;
            }
            tokio::task::yield_now().await;
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("queued {} wanted {want}", gate.queued(key));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn same_key_writes_finish_in_submission_order() {
        let gate = ResourceGate::new();
        let key = ResourceKey::File("same".into());
        let hold = gate
            .acquire(&write_key("same"), &CancellationToken::new())
            .await
            .unwrap();
        let order = Arc::new(Mutex::new(Vec::new()));
        let mut tasks = Vec::new();
        for id in [1_u8, 2] {
            let gate = gate.clone();
            let order = Arc::clone(&order);
            tasks.push(tokio::spawn(async move {
                let guard = gate
                    .acquire(&write_key("same"), &CancellationToken::new())
                    .await
                    .unwrap();
                order.lock().unwrap().push(("start", id));
                tokio::time::sleep(Duration::from_millis(30)).await;
                order.lock().unwrap().push(("end", id));
                drop(guard);
            }));
        }
        until_queued(&gate, &key, 2).await;
        drop(hold);
        for task in tasks {
            task.await.unwrap();
        }
        assert_eq!(
            *order.lock().unwrap(),
            vec![("start", 1), ("end", 1), ("start", 2), ("end", 2)]
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn readers_share_and_a_writer_excludes_them() {
        let gate = ResourceGate::new();
        let first = gate
            .acquire(&read_key("file"), &CancellationToken::new())
            .await
            .unwrap();
        let second = gate
            .acquire(&read_key("file"), &CancellationToken::new())
            .await
            .unwrap();
        let entered = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&entered);
        let gate2 = gate.clone();
        let writer = tokio::spawn(async move {
            let guard = gate2
                .acquire(&write_key("file"), &CancellationToken::new())
                .await
                .unwrap();
            flag.store(true, Ordering::SeqCst);
            drop(guard);
        });
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(
            !entered.load(Ordering::SeqCst),
            "a write ran beside a read of the same key"
        );
        drop((first, second));
        writer.await.unwrap();
        assert!(entered.load(Ordering::SeqCst));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn distinct_write_keys_do_not_block_each_other() {
        let gate = ResourceGate::new();
        let left = gate
            .acquire(&write_key("left"), &CancellationToken::new())
            .await
            .unwrap();
        let right = gate
            .acquire(&write_key("right"), &CancellationToken::new())
            .await
            .unwrap();
        drop((left, right));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn multi_key_acquires_in_sorted_order_do_not_deadlock() {
        let gate = ResourceGate::new();
        let forward = ToolEffects {
            reads: vec![ResourceKey::File("b".into())],
            writes: vec![ResourceKey::File("a".into())],
        };
        let reverse = ToolEffects {
            reads: vec![ResourceKey::File("a".into())],
            writes: vec![ResourceKey::File("b".into())],
        };
        let gate_a = gate.clone();
        let gate_b = gate.clone();
        let first = tokio::spawn(async move {
            let guard = gate_a
                .acquire(&forward, &CancellationToken::new())
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_millis(20)).await;
            drop(guard);
        });
        let second = tokio::spawn(async move {
            let guard = gate_b
                .acquire(&reverse, &CancellationToken::new())
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_millis(20)).await;
            drop(guard);
        });
        tokio::time::timeout(Duration::from_secs(2), async {
            first.await.unwrap();
            second.await.unwrap();
        })
        .await
        .expect("sorted acquisition deadlocked");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cancel_while_queued_does_not_enter() {
        let gate = ResourceGate::new();
        let key = ResourceKey::File("held".into());
        let hold = gate
            .acquire(&write_key("held"), &CancellationToken::new())
            .await
            .unwrap();
        let cancel = CancellationToken::new();
        let gate2 = gate.clone();
        let cancel2 = cancel.clone();
        let waiting = tokio::spawn(async move {
            gate2
                .acquire(&write_key("held"), &cancel2)
                .await
        });
        until_queued(&gate, &key, 1).await;
        cancel.cancel();
        assert!(waiting.await.unwrap().is_err(), "cancelled waiter entered");
        drop(hold);
        let again = gate
            .acquire(&write_key("held"), &CancellationToken::new())
            .await
            .unwrap();
        drop(again);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_waiting_writer_is_not_skipped_by_a_later_reader() {
        let gate = ResourceGate::new();
        let key = ResourceKey::File("fair".into());
        let hold = gate
            .acquire(&read_key("fair"), &CancellationToken::new())
            .await
            .unwrap();
        let order = Arc::new(Mutex::new(Vec::new()));
        let gate_w = gate.clone();
        let order_w = Arc::clone(&order);
        let writer = tokio::spawn(async move {
            let guard = gate_w
                .acquire(&write_key("fair"), &CancellationToken::new())
                .await
                .unwrap();
            order_w.lock().unwrap().push("write");
            drop(guard);
        });
        until_queued(&gate, &key, 1).await;
        let gate_r = gate.clone();
        let order_r = Arc::clone(&order);
        let reader = tokio::spawn(async move {
            let guard = gate_r
                .acquire(&read_key("fair"), &CancellationToken::new())
                .await
                .unwrap();
            order_r.lock().unwrap().push("read");
            drop(guard);
        });
        until_queued(&gate, &key, 2).await;
        drop(hold);
        writer.await.unwrap();
        reader.await.unwrap();
        assert_eq!(*order.lock().unwrap(), vec!["write", "read"]);
    }

    #[test]
    fn file_resource_key_folds_dot_parent_and_separators() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let key = |raw: &str, mode| {
            let resolved = crate::workspace::resolve_agent(root, raw, mode).unwrap();
            file_resource_key(&resolved)
        };
        let all = ToolPathMode::All;
        let dot = crate::workspace::resolve_agent(root, "./dot.txt", all).unwrap();
        let plain = crate::workspace::resolve_agent(root, "dot.txt", all).unwrap();
        assert_ne!(
            dot.to_string_lossy(),
            plain.to_string_lossy(),
            "All mode keeps the lexical dot; the resource key has to drop it"
        );
        assert_eq!(key("./dot.txt", all), key("dot.txt", all));
        let lexical = root.join("gone").join("..").join("parented.txt");
        assert!(
            lexical
                .components()
                .any(|component| matches!(component, Component::ParentDir)),
            "the lexical path must still contain '..'"
        );
        assert_eq!(
            file_resource_key(&lexical),
            file_resource_key(&root.join("parented.txt"))
        );
        #[cfg(windows)]
        {
            let safe = ToolPathMode::Safe;
            for mode in [all, safe] {
                let lower = crate::workspace::resolve_agent(root, "case.txt", mode).unwrap();
                let upper = crate::workspace::resolve_agent(root, "Case.txt", mode).unwrap();
                assert_ne!(
                    lower.to_string_lossy().into_owned(),
                    upper.to_string_lossy().into_owned(),
                    "resolve keeps case; the resource key has to fold it"
                );
                assert_eq!(key("case.txt", mode), key("Case.txt", mode));
                assert_eq!(key("sub/nested.txt", mode), key(r"sub\nested.txt", mode));
                assert_eq!(key("slash.txt", mode), key(r".\slash.txt", mode));
                assert_eq!(key("trail.txt", mode), key("trail.txt.", mode));
                assert_eq!(key("trail.txt", mode), key("trail.txt ", mode));
                assert_eq!(key("ä.txt", mode), key("Ä.txt", mode));
                let ResourceKey::File(folded) = key("Case.txt", mode) else {
                    panic!("file key");
                };
                assert!(
                    folded.chars().all(|ch| !ch.is_ascii_lowercase()),
                    "file-system case fold uppercases ASCII letters: {folded}"
                );
            }
            let abs = format!(r"{}\gone\..\parented.txt", root.display());
            let parented = crate::workspace::resolve_agent(root, &abs, all).unwrap();
            let plain = crate::workspace::resolve_agent(root, "parented.txt", all).unwrap();
            assert_ne!(
                parented.to_string_lossy().into_owned(),
                plain.to_string_lossy().into_owned(),
                "resolve keeps '..'; the resource key has to pop it"
            );
            assert_eq!(file_resource_key(&parented), file_resource_key(&plain));
            assert_eq!(
                file_resource_key(Path::new(r"C:\..\..\parented.txt")),
                file_resource_key(Path::new(r"C:\parented.txt"))
            );
        }
    }

    #[test]
    fn five_tools_declare_shared_reads_and_exclusive_writes() {
        use crate::tool::Tool;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mode = ToolPathMode::All;
        let read = crate::tools::read::ReadTool::default().effects(
            &serde_json::json!({"path": "a.txt"}),
            mode,
            root,
        );
        let write = crate::tools::write::WriteTool::new().effects(
            &serde_json::json!({"path": "a.txt"}),
            mode,
            root,
        );
        let edit = crate::tools::edit::EditTool::new().effects(
            &serde_json::json!({"path": "a.txt", "old_string": "a", "new_string": "b"}),
            mode,
            root,
        );
        assert!(read.writes.is_empty() && write.reads.is_empty() && edit.reads.is_empty());
        assert_eq!(read.reads, write.writes);
        assert_eq!(write.writes, edit.writes);
        let glob = crate::tools::glob::GlobTool.effects(
            &serde_json::json!({"pattern": "*.rs"}),
            mode,
            root,
        );
        let grep = crate::tools::grep::GrepTool.effects(
            &serde_json::json!({"pattern": "x", "path": "src"}),
            mode,
            root,
        );
        assert_eq!(glob.reads, vec![file_resource_key(root)]);
        assert!(glob.writes.is_empty());
        assert_eq!(grep.reads, vec![lock_key(root, "src", mode)]);
        assert!(grep.writes.is_empty());
    }
}

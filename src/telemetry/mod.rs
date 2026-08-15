//! Process stats and live tracing broadcast for Web status bar.

mod log_broadcast;

use std::sync::OnceLock;

pub use log_broadcast::log_broadcast_layer;
use tokio::sync::broadcast;

use crate::client_protocol::protocol::LogLine;

const LOG_CHANNEL_CAPACITY: usize = 256;

static LOG_TX: OnceLock<broadcast::Sender<LogLine>> = OnceLock::new();

fn log_sender() -> &'static broadcast::Sender<LogLine> {
    LOG_TX.get_or_init(|| {
        let (tx, _) = broadcast::channel(LOG_CHANNEL_CAPACITY);
        tx
    })
}

/// Subscribe to live tracing events (used when a Web client expands the log panel).
pub fn subscribe_logs() -> broadcast::Receiver<LogLine> {
    log_sender().subscribe()
}

pub(crate) fn publish_log(line: LogLine) {
    let _ = log_sender().send(line);
}

/// Best-effort return of freed heap pages to the OS (glibc `malloc_trim`).
pub fn release_heap_to_os() {
    #[cfg(target_os = "linux")]
    {
        // SAFETY: `malloc_trim` is only compiled on Linux with glibc,
        // where the symbol is guaranteed to exist. `pad=0` is documented
        // as a safe value that releases all freeable heap memory.
        unsafe extern "C" {
            fn malloc_trim(pad: usize) -> i32;
        }
        // SAFETY: `malloc_trim` with `pad=0` is a safe glibc API call
        // that only releases free heap pages to the OS.
        unsafe {
            let _ = malloc_trim(0);
        }
    }
}

/// Resident set size in kilobytes.
///
/// Linux: `VmRSS` from `/proc/self/status`.  
/// Windows: working set via `GetProcessMemoryInfo` (bytes → KiB).
pub fn read_rss_kb() -> Option<u64> {
    read_rss_kb_for_pid(std::process::id())
}

/// Resident set size for a process (see [`read_rss_kb`] for platform notes).
pub fn read_rss_kb_for_pid(pid: u32) -> Option<u64> {
    #[cfg(unix)]
    {
        proc_status_kb_field_for_pid(pid, "VmRSS:")
    }
    #[cfg(windows)]
    {
        windows_working_set_kb(pid)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
        None
    }
}

/// Sum of RSS for multiple PIDs (dead/unknown PIDs are skipped).
pub fn sum_rss_kb_for_pids(pids: &[u32]) -> Option<u64> {
    if pids.is_empty() {
        return Some(0);
    }
    let mut sum = 0u64;
    let mut any = false;
    for pid in pids {
        if let Some(kb) = read_rss_kb_for_pid(*pid) {
            sum = sum.saturating_add(kb);
            any = true;
        }
    }
    any.then_some(sum)
}

/// Memory breakdown for the status bar (core + tracked child processes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemorySample {
    pub core_kb: Option<u64>,
    pub embed_kb: Option<u64>,
    pub lsp_kb: Option<u64>,
}

impl MemorySample {
    pub fn total_kb(&self) -> Option<u64> {
        let core = self.core_kb?;
        Some(
            core.saturating_add(self.embed_kb.unwrap_or(0))
                .saturating_add(self.lsp_kb.unwrap_or(0)),
        )
    }
}

pub fn sample_memory(embed_pids: &[u32], lsp_pids: &[u32]) -> MemorySample {
    MemorySample {
        core_kb: read_rss_kb(),
        embed_kb: sum_rss_kb_for_pids(embed_pids),
        lsp_kb: sum_rss_kb_for_pids(lsp_pids),
    }
}

#[cfg(unix)]
fn proc_status_kb_field_for_pid(pid: u32, prefix: &str) -> Option<u64> {
    let path = format!("/proc/{pid}/status");
    std::fs::read_to_string(path)
        .ok()?
        .lines()
        .find(|l| l.starts_with(prefix))?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}

/// Working set size in KiB (`GetProcessMemoryInfo` → `WorkingSetSize`).
#[cfg(windows)]
fn windows_working_set_kb(pid: u32) -> Option<u64> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    // SAFETY: Win32 process-query APIs; pseudo-handle from GetCurrentProcess
    // must not be ClosedHandle'd; OpenProcess handles are closed below.
    // Child processes: ask for VM_READ as well — some hosts still expect it for
    // GetProcessMemoryInfo even though modern docs allow LIMITED alone.
    unsafe {
        let (handle, owned) = if pid == std::process::id() {
            (GetCurrentProcess(), false)
        } else {
            use windows_sys::Win32::System::Threading::PROCESS_VM_READ;
            let access = PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ;
            let h = OpenProcess(access, 0, pid);
            if h.is_null() {
                return None;
            }
            (h, true)
        };

        let mut counters = std::mem::zeroed::<PROCESS_MEMORY_COUNTERS>();
        counters.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        let ok = GetProcessMemoryInfo(handle, &mut counters, counters.cb);
        if owned {
            CloseHandle(handle);
        }
        if ok == 0 {
            return None;
        }
        Some((counters.WorkingSetSize as u64) / 1024)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_rss_kb_positive_on_linux() {
        if std::path::Path::new("/proc/self/status").exists() {
            let rss = read_rss_kb().expect("VmRSS on linux");
            assert!(rss > 0, "rss should be positive, got {rss}");
            let pid_rss = read_rss_kb_for_pid(std::process::id()).expect("pid rss");
            assert!(pid_rss > 0);
        }
    }

    #[test]
    #[cfg(windows)]
    fn read_rss_kb_positive_on_windows() {
        let rss = read_rss_kb().expect("working set on windows");
        assert!(rss > 0, "rss should be positive, got {rss}");
        let pid_rss = read_rss_kb_for_pid(std::process::id()).expect("pid rss");
        assert!(pid_rss > 0);
        assert!(
            read_rss_kb_for_pid(u32::MAX).is_none(),
            "unknown pid should yield None"
        );
    }

    #[test]
    fn sum_rss_empty_is_zero() {
        assert_eq!(sum_rss_kb_for_pids(&[]), Some(0));
    }
}

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

/// Memory footprint of this process in kilobytes (see [`read_rss_kb_for_pid`]
/// for the platform-specific metric).
pub fn read_rss_kb() -> Option<u64> {
    read_rss_kb_for_pid(std::process::id())
}

/// Memory footprint for a process in kilobytes.
///
/// Linux: `VmRSS` from `/proc/<pid>/status`.  
/// Windows: private committed bytes (`GetProcessMemoryInfo` → `PrivateUsage`).
/// The working set is deliberately NOT used: the kernel trims it aggressively,
/// so a process holding ~2.5 GB of commit (e.g. rust-analyzer) can show a
/// ~130 MB working set, hiding real memory pressure from the status bar.
pub fn read_rss_kb_for_pid(pid: u32) -> Option<u64> {
    #[cfg(unix)]
    {
        proc_status_kb_field_for_pid(pid, "VmRSS:")
    }
    #[cfg(windows)]
    {
        windows_private_kb(pid)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
        None
    }
}

/// Expand root PIDs to the full set of descendant PIDs (roots included).
///
/// Language servers are frequently launched through wrapper processes
/// (rustup shims, node/nvm wrappers, ...) that spawn the real server as a
/// child and forward stdio. Measuring only the direct child under-reports
/// memory by orders of magnitude, so telemetry sums the whole subtree.
///
/// Unknown/dead roots are kept in the output and simply skipped later by
/// [`sum_rss_kb_for_pids`].
pub fn descendant_pids(roots: &[u32]) -> Vec<u32> {
    let map = process_ppid_map();
    descendants_from_ppid_map(&map, roots)
}

/// Pure BFS over a parent→children map; the visited set bounds the walk and
/// guards against cycles caused by PID reuse.
fn descendants_from_ppid_map(map: &std::collections::HashMap<u32, Vec<u32>>, roots: &[u32]) -> Vec<u32> {
    let mut seen: std::collections::HashSet<u32> = roots.iter().copied().collect();
    let mut queue: std::collections::VecDeque<u32> = roots.iter().copied().collect();
    let mut out = Vec::new();
    while let Some(pid) = queue.pop_front() {
        out.push(pid);
        if let Some(children) = map.get(&pid) {
            for &child in children {
                if seen.insert(child) {
                    queue.push_back(child);
                }
            }
        }
    }
    out
}

/// Snapshot of the system process table as a parent→children PID map.
#[cfg(windows)]
fn process_ppid_map() -> std::collections::HashMap<u32, Vec<u32>> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };

    let mut map = std::collections::HashMap::new();
    // SAFETY: Win32 Toolhelp snapshot APIs. The snapshot handle is closed on
    // every path below; iteration stops when Process32NextW returns 0.
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return map;
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        if Process32FirstW(snap, &mut entry) != 0 {
            loop {
                map.entry(entry.th32ParentProcessID)
                    .or_insert_with(Vec::new)
                    .push(entry.th32ProcessID);
                if Process32NextW(snap, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snap);
    }
    map
}

/// Snapshot of the system process table as a parent→children PID map.
#[cfg(unix)]
fn process_ppid_map() -> std::collections::HashMap<u32, Vec<u32>> {
    let mut map = std::collections::HashMap::new();
    let Ok(dir) = std::fs::read_dir("/proc") else {
        return map;
    };
    for entry in dir.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Ok(pid) = name.parse::<u32>() else {
            continue;
        };
        let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else {
            continue;
        };
        // comm may contain spaces and parens; fields restart after the last ')'.
        // Format: "pid (comm) state ppid ..." → ppid is the 2nd field after ')'.
        let Some((_, rest)) = stat.rsplit_once(')') else {
            continue;
        };
        let Some(ppid) = rest.split_whitespace().nth(1).and_then(|f| f.parse::<u32>().ok())
        else {
            continue;
        };
        map.entry(ppid).or_insert_with(Vec::new).push(pid);
    }
    map
}

#[cfg(not(any(unix, windows)))]
fn process_ppid_map() -> std::collections::HashMap<u32, Vec<u32>> {
    std::collections::HashMap::new()
}

/// Sum of memory footprint for multiple PIDs (dead/unknown PIDs are skipped).
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
    // Expand each tracked root into its full process subtree so wrapper
    // processes (rustup shims etc.) do not hide the real server's memory.
    let embed_tree = descendant_pids(embed_pids);
    let lsp_tree = descendant_pids(lsp_pids);
    MemorySample {
        core_kb: read_rss_kb(),
        embed_kb: sum_rss_kb_for_pids(&embed_tree),
        lsp_kb: sum_rss_kb_for_pids(&lsp_tree),
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

/// Private committed memory in KiB (`GetProcessMemoryInfo` on
/// `PROCESS_MEMORY_COUNTERS_EX` → `PrivateUsage`).
#[cfg(windows)]
fn windows_private_kb(pid: u32) -> Option<u64> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
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

        // Pass the EX struct with its larger cb so the API fills PrivateUsage.
        let mut counters = std::mem::zeroed::<PROCESS_MEMORY_COUNTERS_EX>();
        counters.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
        let ok = GetProcessMemoryInfo(
            handle,
            &mut counters as *mut PROCESS_MEMORY_COUNTERS_EX as *mut PROCESS_MEMORY_COUNTERS,
            counters.cb,
        );
        if owned {
            CloseHandle(handle);
        }
        if ok == 0 {
            return None;
        }
        Some((counters.PrivateUsage as u64) / 1024)
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

    fn ppid_map(pairs: &[(u32, u32)]) -> std::collections::HashMap<u32, Vec<u32>> {
        // (ppid, pid)
        let mut map: std::collections::HashMap<u32, Vec<u32>> =
            std::collections::HashMap::new();
        for &(ppid, pid) in pairs {
            map.entry(ppid).or_default().push(pid);
        }
        map
    }

    #[test]
    fn descendants_include_root_and_transitive_children() {
        // 1 → 2 → 3, plus 1 → 4
        let map = ppid_map(&[(1, 2), (2, 3), (1, 4)]);
        let mut got = descendants_from_ppid_map(&map, &[1]);
        got.sort_unstable();
        assert_eq!(got, vec![1, 2, 3, 4]);
    }

    #[test]
    fn descendants_survive_pid_reuse_cycles() {
        // PID reuse can make the ppid map cyclic: 10 → 11 → 10.
        let map = ppid_map(&[(10, 11), (11, 10)]);
        let mut got = descendants_from_ppid_map(&map, &[10]);
        got.sort_unstable();
        assert_eq!(got, vec![10, 11]);
    }

    #[test]
    fn descendants_skip_unrelated_subtrees() {
        let map = ppid_map(&[(1, 2), (9, 8)]);
        let mut got = descendants_from_ppid_map(&map, &[1]);
        got.sort_unstable();
        assert_eq!(got, vec![1, 2]);
    }

    #[test]
    fn descendants_empty_roots_is_empty() {
        let map = ppid_map(&[(1, 2)]);
        assert!(descendants_from_ppid_map(&map, &[]).is_empty());
    }

    #[test]
    fn descendant_pids_sees_spawned_child() {
        // Live check that the platform snapshot really observes children:
        // spawn a short-lived child and require it in our subtree.
        #[cfg(windows)]
        let mut child = std::process::Command::new("ping")
            .args(["-n", "3", "127.0.0.1"])
            .spawn()
            .expect("spawn ping");
        #[cfg(unix)]
        let mut child = std::process::Command::new("sleep")
            .arg("1")
            .spawn()
            .expect("spawn sleep");
        #[cfg(not(any(unix, windows)))]
        let mut child = std::process::Command::new("true").spawn().expect("spawn");

        let pid = std::process::id();
        let found = descendant_pids(&[pid]).contains(&child.id());
        let _ = child.kill();
        let _ = child.wait();
        assert!(found, "spawned child should appear in our process subtree");
    }
}

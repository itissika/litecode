//! Child-process lifetime binding.
//!
//! Graceful serve shutdown already stops language servers and workers
//! (`serve::router::listen` → `WorkspaceEngines::stop_all`). The gap is a
//! non-graceful death of this process itself: crash, `taskkill`, or a
//! force-closed console skips that cleanup entirely, and on Windows the
//! kernel does not cascade-kill children — leaked rust-analyzer instances
//! have survived for hours holding tens of GB of commit memory.
//!
//! - Windows: every spawned helper is assigned to one process-wide Job
//!   Object created with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`. When this
//!   process terminates for *any* reason the job handle closes and the
//!   kernel kills all member processes. This also closes the stdin-EOF
//!   escape hatch on Windows, where an inherited pipe write-handle can keep
//!   the child's stdin open forever.
//! - Unix: the stdio pipe already provides the tie (the child sees stdin
//!   EOF when this process dies, and language servers exit on EOF); fd
//!   inheritance is per-descriptor, so siblings cannot keep it open.
//!   `PR_SET_PDEATHSIG` is deliberately *not* used: it fires when the
//!   forking *thread* exits, and spawns here happen on runtime worker
//!   threads that may be retired while the process lives.
//!
//! [`sweep_orphan_lsp_processes`] cleans up leftovers from instances that
//! leaked before this binding existed: at serve startup it kills
//! language-server processes installed under the managed `lsp` directory
//! whose parent process is gone.

/// Bind a freshly spawned child process so it cannot outlive this process.
///
/// Best-effort: failures are logged and do not affect the child. Must be
/// called shortly after spawn (before the pid can be reused).
pub fn bind_child_to_parent(pid: u32) {
    #[cfg(windows)]
    {
        bind_child_to_job(pid);
    }
    #[cfg(not(windows))]
    {
        let _ = pid;
    }
}

/// Kill language-server processes from the managed install directory whose
/// parent process is dead. Runs a few passes so grandchildren (e.g.
/// `rust-analyzer-proc-macro-srv` under a leaked rust-analyzer) are reaped
/// once their own parent dies. Best-effort; never kills processes with a
/// live parent (other running litecode instances keep their servers).
pub fn sweep_orphan_lsp_processes() {
    for pass in 0..3 {
        let killed = sweep_once();
        if killed == 0 {
            return;
        }
        tracing::info!(pass, killed, "orphaned language-server sweep pass");
    }
}

#[cfg(windows)]
fn bind_child_to_job(pid: u32) {
    use std::sync::OnceLock;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        SetInformationJobObject,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE,
    };

    // Stored as usize: HANDLE is a raw pointer and therefore !Sync.
    static JOB: OnceLock<usize> = OnceLock::new();

    fn kill_on_close_job() -> Option<HANDLE> {
        let handle = *JOB.get_or_init(|| unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                return 0;
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let ok = SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const core::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            if ok == 0 {
                CloseHandle(job);
                return 0;
            }
            job as usize
        });
        if handle != 0 {
            Some(handle as HANDLE)
        } else {
            None
        }
    }

    if pid == 0 {
        return;
    }
    let Some(job) = kill_on_close_job() else {
        tracing::warn!("could not create kill-on-close job object; children may leak on crash");
        return;
    };
    // SAFETY: straightforward Win32 calls; both handles are closed below.
    unsafe {
        let proc = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid);
        if proc.is_null() {
            return;
        }
        let ok = AssignProcessToJobObject(job, proc);
        CloseHandle(proc);
        if ok == 0 {
            tracing::warn!(
                error = std::io::Error::last_os_error().to_string(),
                pid,
                "AssignProcessToJobObject failed; child may leak on crash"
            );
        }
    }
}

/// One sweep pass. Returns how many processes were killed.
#[cfg(windows)]
fn sweep_once() -> usize {
    use std::path::Path;

    use crate::lsp::paths::lsp_dir;
    use crate::serve::shutdown::{is_process_alive, kill_process};
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };

    fn exe_under(pid: u32, dir: &Path) -> bool {
        // SAFETY: Win32 query calls; the opened handle is closed on every path.
        unsafe {
            let proc = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if proc.is_null() {
                return false;
            }
            let mut buf = [0u16; 1024];
            let mut len = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(proc, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len);
            CloseHandle(proc);
            if ok == 0 {
                return false;
            }
            let exe = String::from_utf16_lossy(&buf[..len as usize]);
            path_starts_with_case_insensitive(Path::new(&exe), dir)
        }
    }

    fn path_starts_with_case_insensitive(path: &Path, base: &Path) -> bool {
        let norm = |p: &Path| {
            crate::config::path::strip_verbatim(p)
                .to_string_lossy()
                .to_lowercase()
        };
        let (path, base) = (norm(path), norm(base));
        Path::new(&path).starts_with(&base)
    }

    let Ok(dir) = lsp_dir() else {
        return 0;
    };
    let self_pid = std::process::id();

    let mut orphans = Vec::new();
    // SAFETY: Win32 Toolhelp snapshot APIs. The snapshot handle is closed on
    // every path below; iteration stops when Process32NextW returns 0.
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return 0;
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        if Process32FirstW(snap, &mut entry) != 0 {
            loop {
                let pid = entry.th32ProcessID;
                let ppid = entry.th32ParentProcessID;
                if pid != self_pid && !is_process_alive(ppid) && exe_under(pid, &dir) {
                    orphans.push(pid);
                }
                if Process32NextW(snap, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snap);
    }

    for pid in &orphans {
        tracing::warn!(pid, "killing orphaned language-server process (dead parent)");
        kill_process(*pid);
    }
    orphans.len()
}

/// One sweep pass. Returns how many processes were killed.
#[cfg(target_os = "linux")]
fn sweep_once() -> usize {
    use crate::lsp::paths::lsp_dir;
    use crate::serve::shutdown::kill_process;

    let Ok(dir) = lsp_dir() else {
        return 0;
    };
    let self_pid = std::process::id();
    let mut killed = 0usize;

    let Ok(proc_dir) = std::fs::read_dir("/proc") else {
        return 0;
    };
    for entry in proc_dir.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Ok(pid) = name.parse::<u32>() else {
            continue;
        };
        if pid == self_pid {
            continue;
        }
        let Ok(exe) = std::fs::read_link(format!("/proc/{pid}/exe")) else {
            continue;
        };
        if !exe.starts_with(&dir) {
            continue;
        }
        // On Linux a dead parent means the child was reparented to init (or a
        // subreaper); ppid == 1 is the reliable orphan signal.
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            continue;
        };
        let Some((_, rest)) = stat.rsplit_once(')') else {
            continue;
        };
        let Some(ppid) = rest.split_whitespace().nth(1).and_then(|f| f.parse::<u32>().ok())
        else {
            continue;
        };
        if ppid == 1 {
            tracing::warn!(pid, "killing orphaned language-server process (dead parent)");
            kill_process(pid);
            killed += 1;
        }
    }
    killed
}

#[cfg(not(any(windows, target_os = "linux")))]
fn sweep_once() -> usize {
    0
}

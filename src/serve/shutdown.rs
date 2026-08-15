use std::time::Duration;

#[derive(Debug, Clone, Copy, Default)]
pub struct ShutdownWatch {
    pub parent_pid: Option<u32>,
    /// Only enable when the host keeps stdin open as a keepalive pipe.
    /// Default off: non-interactive launches often have stdin already at EOF.
    pub stdin_eof: bool,
}

/// Returns whether `pid` refers to a live process.
pub fn is_process_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        unsafe { libc::kill(pid as i32, 0) == 0 }
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
        use windows_sys::Win32::System::Threading::{
            GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };

        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if handle.is_null() {
                return false;
            }
            let mut exit_code = 0u32;
            let ok = GetExitCodeProcess(handle, &mut exit_code);
            CloseHandle(handle);
            ok != 0 && exit_code == STILL_ACTIVE as u32
        }
    }
}

/// Wait until any configured shutdown signal fires.
pub async fn wait_for_shutdown(watch: ShutdownWatch) {
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("serve shutting down (ctrl_c)");
        }
        _ = wait_for_stdin_eof(watch.stdin_eof) => {
            tracing::info!("serve shutting down (stdin EOF)");
        }
        _ = wait_for_parent_exit(watch.parent_pid) => {
            tracing::info!("serve shutting down (parent exited)");
        }
    }
}

async fn wait_for_stdin_eof(enabled: bool) {
    if !enabled {
        std::future::pending::<()>().await;
        return;
    }
    use tokio::io::AsyncReadExt;
    let mut stdin = tokio::io::stdin();
    let mut buf = [0u8; 256];
    loop {
        match stdin.read(&mut buf).await {
            Ok(0) => break,
            Ok(_) => continue,
            Err(_) => break,
        }
    }
}

async fn wait_for_parent_exit(parent_pid: Option<u32>) {
    let Some(pid) = parent_pid else {
        std::future::pending::<()>().await;
        return;
    };
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        if !is_process_alive(pid) {
            break;
        }
    }
}

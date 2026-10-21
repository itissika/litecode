//! TerminalHub PTY contract — real FS / ConPTY.

use std::time::{Duration, Instant};

use litecode::config::path::canon_abs;
use litecode::config::resolved::WorkspacePaths;
use litecode::config::workspace::{clear_runtime_paths, set_runtime_paths};
use litecode::terminal::{ConnectionId, CreateOptions, TerminalHub};

/// Mirror of `find_git_bash` in src/terminal/shell.rs — used to pick
/// shell-appropriate commands for the Windows branches below.
#[cfg(windows)]
fn windows_uses_git_bash() -> bool {
    use std::path::PathBuf;
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(pf) = std::env::var_os("ProgramFiles") {
        candidates.push(PathBuf::from(pf).join("Git").join("bin").join("bash.exe"));
    }
    if let Some(pf) = std::env::var_os("ProgramFiles(x86)") {
        candidates.push(PathBuf::from(pf).join("Git").join("bin").join("bash.exe"));
    }
    candidates.push(PathBuf::from(r"C:\Program Files\Git\bin\bash.exe"));
    candidates.push(PathBuf::from(r"C:\Program Files (x86)\Git\bin\bash.exe"));
    candidates.iter().any(|p| p.exists())
}

/// Interactive sessions use their owner's directed stream.
///
/// Human PTYs no longer answer cursor-position reports; xterm does. This
/// test has no emulator, and both PowerShell and Git Bash block on DSR
/// before they print, so the loop replies `\x1b[1;1R` itself.
fn wait_for_terminal_data(
    hub: &TerminalHub,
    caller: &ConnectionId,
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<litecode::terminal::TerminalEvent>,
    id: &str,
    needle: &str,
    timeout: Duration,
) -> String {
    use litecode::terminal::{TerminalEvent, TerminalEventKind};
    let start = Instant::now();
    let mut acc = String::new();
    let mut replied = 0usize;
    while start.elapsed() < timeout {
        while let Ok(TerminalEvent { id: ev_id, kind }) = rx.try_recv() {
            if ev_id == id {
                if let TerminalEventKind::Data(d) = kind {
                    acc.push_str(&d);
                    // Count across the whole buffer so a report split on a
                    // read boundary still gets exactly one reply.
                    let seen = acc.matches("\u{1b}[6n").count();
                    while replied < seen {
                        let _ = hub.write(caller, id, b"\x1b[1;1R");
                        replied += 1;
                    }
                }
            }
        }
        if acc.contains(needle) {
            return acc;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    acc
}

#[test]
fn interactive_echo_round_trip() {
    let hub = TerminalHub::new();
    let caller = ConnectionId::new();
    let mut events = hub.attach_connection(caller.clone());
    let dir = tempfile::tempdir().unwrap();
    let root = canon_abs(dir.path()).unwrap();
    set_runtime_paths(WorkspacePaths::for_legacy_root(&root));

    let id = hub
        .create(
            &caller,
            CreateOptions {
                cols: 80,
                rows: 24,
                cwd: Some(root.clone()),
            },
        )
        .expect("create interactive")
        .id;

    #[cfg(windows)]
    let cmd: &[u8] = if windows_uses_git_bash() {
        b"echo LITECODE_PTY_OK\r"
    } else {
        b"Write-Output 'LITECODE_PTY_OK'\r"
    };
    #[cfg(not(windows))]
    let cmd = b"echo LITECODE_PTY_OK\n";

    hub.write(&caller, &id, cmd).expect("write");
    let out = wait_for_terminal_data(
        &hub,
        &caller,
        &mut events,
        &id,
        "LITECODE_PTY_OK",
        Duration::from_secs(10),
    );
    assert!(
        out.contains("LITECODE_PTY_OK"),
        "expected marker in broadcast output, got: {out:?}"
    );
    // REV-7: interactive sessions must not buffer output (memory stays bounded).
    assert!(
        hub.take_output(&id).map_or(true, |s| s.is_empty()),
        "interactive output must not accumulate in the buffer"
    );

    hub.resize(&caller, &id, 100, 30).expect("resize");
    hub.close(&caller, &id).expect("close");
    clear_runtime_paths();
}

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
fn wait_for_terminal_data(
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<litecode::terminal::TerminalEvent>,
    id: &str,
    needle: &str,
    timeout: Duration,
) -> String {
    use litecode::terminal::{TerminalEvent, TerminalEventKind};
    let start = Instant::now();
    let mut acc = String::new();
    while start.elapsed() < timeout {
        while let Ok(TerminalEvent { id: ev_id, kind }) = rx.try_recv() {
            if ev_id == id {
                if let TerminalEventKind::Data(d) = kind {
                    acc.push_str(&d);
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
        .expect("create interactive");

    #[cfg(windows)]
    let cmd: &[u8] = if windows_uses_git_bash() {
        b"echo LITECODE_PTY_OK\r"
    } else {
        b"Write-Output 'LITECODE_PTY_OK'\r"
    };
    #[cfg(not(windows))]
    let cmd = b"echo LITECODE_PTY_OK\n";

    hub.write(&caller, &id, cmd).expect("write");
    let out = wait_for_terminal_data(&mut events, &id, "LITECODE_PTY_OK", Duration::from_secs(10));
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

#[test]
fn exec_once_runs_in_lap_cwd() {
    let hub = TerminalHub::new();
    let dir = tempfile::tempdir().unwrap();
    let root = canon_abs(dir.path()).unwrap();
    set_runtime_paths(WorkspacePaths::for_legacy_root(&root));

    #[cfg(windows)]
    let command: &str = if windows_uses_git_bash() {
        "pwd -W"
    } else {
        "(Get-Location).Path"
    };
    #[cfg(not(windows))]
    let command = "pwd";

    let result = hub
        .exec_once(command, Some(&root), Duration::from_secs(15), None, &root)
        .expect("exec_once");

    let root_s = root.to_string_lossy();
    assert!(
        !result.output.contains(r"\\?\"),
        "output must not contain verbatim prefix: {}",
        result.output
    );
    // PowerShell / pwd may differ on slash style; compare case-insensitively on Windows.
    let hay = result.output.replace('/', "\\");
    let needle = root_s.replace('/', "\\");
    #[cfg(windows)]
    {
        assert!(
            hay.to_ascii_lowercase()
                .contains(&needle.to_ascii_lowercase()),
            "cwd output should mention LAP root {needle}, got {}",
            result.output
        );
    }
    #[cfg(not(windows))]
    {
        assert!(
            result.output.contains(root_s.as_ref()),
            "cwd output should mention LAP root {root_s}, got {}",
            result.output
        );
    }
    clear_runtime_paths();
}

#[test]
fn exec_once_returns_real_exit_code_for_success() {
    // 3.3 (phase E): a successful command must report a real exit code (0),
    // never the spurious `None`/`-1` that could occur if the reader hit EOF
    // before the child status was reaped.
    let hub = TerminalHub::new();
    let dir = tempfile::tempdir().unwrap();
    let root = canon_abs(dir.path()).unwrap();
    set_runtime_paths(WorkspacePaths::for_legacy_root(&root));

    #[cfg(windows)]
    let command: &str = if windows_uses_git_bash() {
        "exit 0"
    } else {
        "exit 0"
    };
    #[cfg(not(windows))]
    let command = "exit 0";

    let result = hub
        .exec_once(command, Some(&root), Duration::from_secs(15), None, &root)
        .expect("exec_once should succeed");
    assert_eq!(
        result.exit_code,
        Some(0),
        "successful command must yield exit_code 0, got {:?}",
        result.exit_code
    );
    clear_runtime_paths();
}

#[test]
fn spawn_command_background_writes_output_file_and_kill() {
    let hub = TerminalHub::new();
    let dir = tempfile::tempdir().unwrap();
    let root = canon_abs(dir.path()).unwrap();
    set_runtime_paths(WorkspacePaths::for_legacy_root(&root));

    #[cfg(windows)]
    let command: &str = if windows_uses_git_bash() {
        "echo LITECODE_BG_OK; sleep 30"
    } else {
        "Write-Output 'LITECODE_BG_OK'; Start-Sleep -Seconds 30"
    };
    #[cfg(not(windows))]
    let command = "echo LITECODE_BG_OK; sleep 30";

    let spawned = hub
        .spawn_command(command, Some(root.as_path()), root.as_path(), "test")
        .expect("spawn");
    assert!(
        spawned.output_path.exists(),
        "output file should be created: {}",
        spawned.output_path.display()
    );
    assert!(
        spawned.output_path.to_string_lossy().contains(".litecode"),
        "output should live under .litecode: {}",
        spawned.output_path.display()
    );

    let start = Instant::now();
    let mut body = String::new();
    while start.elapsed() < Duration::from_secs(10) {
        body = std::fs::read_to_string(&spawned.output_path).unwrap_or_default();
        if body.contains("LITECODE_BG_OK") {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        body.contains("LITECODE_BG_OK"),
        "expected marker in output file, got: {body:?}"
    );

    let info = hub.kill(&spawned.id).expect("kill");
    assert!(
        !info.alive || info.exit_code.is_some() || !hub.session_info(&spawned.id).unwrap().alive
    );
    assert_eq!(info.output_path.as_ref(), Some(&spawned.output_path));
    let _ = hub.close_agent(&spawned.id);
    // File retained after kill/close for read-tool consumption.
    assert!(spawned.output_path.exists());
    clear_runtime_paths();
}

//! LSP Hub: single language-server process pool shared by Agent tool and Editor.

pub mod conn;
pub mod deps;
pub mod format;
pub mod hub;
pub mod install;
pub mod paths;
pub mod project_root;
pub mod server;
pub mod server_map;
pub mod status;
pub mod sweep_gate;
pub mod uri;

pub use conn::LspDiagnosticEvent;
pub use conn::LspReply;
pub use hub::{LspDiagFeedback, LspHub, LspToolOutput, SharedLspHub};
pub use server_map::{
    check_workspace_dependencies, command_parts, detect_needed_server_commands,
    program_from_command, server_command_for_ext, server_map,
};
pub use status::{LspInstanceStatus, LspLifecycle};
pub use uri::file_to_uri;

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;

    use crate::lsp::uri::{
        normalize_windows_file_uri, publish_diagnostics_uri_matches, uri_to_path,
    };

    #[test]
    fn normalize_windows_verbatim_file_uri() {
        assert_eq!(
            normalize_windows_file_uri("file:////?/E:/litecode/src/agent/core.rs"),
            "file:///E:/litecode/src/agent/core.rs"
        );
        assert_eq!(
            normalize_windows_file_uri("file:///?/E:/litecode/foo.rs"),
            "file:///E:/litecode/foo.rs"
        );
        assert_eq!(
            normalize_windows_file_uri("file:////?/UNC/host/share/a.rs"),
            "file://host/share/a.rs"
        );
        // Unrelated URIs pass through unchanged.
        assert_eq!(
            normalize_windows_file_uri("file:///home/proj/a.rs"),
            "file:///home/proj/a.rs"
        );
        assert_eq!(
            normalize_windows_file_uri("file:///E:/litecode/a.rs"),
            "file:///E:/litecode/a.rs"
        );
    }

    /// Drive letters are case-insensitive only on Windows; on Unix `/E:` and
    /// `/e:` are distinct directories, so this half is Windows-only.
    #[cfg(windows)]
    #[test]
    fn publish_diagnostics_uri_matches_drive_case() {
        assert!(publish_diagnostics_uri_matches(
            "file:///E:/litecode/src/a.rs",
            "file:///e:/litecode/src/a.rs"
        ));
    }

    /// Verbatim-URI recovery is string-level and holds on every platform.
    #[test]
    fn publish_diagnostics_uri_matches_mangled_verbatim() {
        assert!(publish_diagnostics_uri_matches(
            "file:////?/E:/litecode/src/a.rs",
            "file:///E:/litecode/src/a.rs"
        ));
    }

    #[test]
    fn file_to_uri_strips_verbatim_prefix() {
        let uri = file_to_uri(Path::new(r"\\?\E:\litecode\src\a.rs"));
        assert!(
            !uri.contains("?/"),
            "uri must not retain verbatim marker: {uri}"
        );
        assert!(
            uri.contains("E:/litecode/src/a.rs") || uri.contains("E:\\litecode\\src\\a.rs"),
            "uri={uri}"
        );
    }

    #[test]
    fn uri_to_path_recovers_mangled_verbatim_file_uri() {
        // Platform-independent: the mangled URI is rewritten and parses.
        let mangled = "file:////?/E:/litecode/src/agent/core.rs";
        assert_eq!(
            crate::lsp::uri::normalize_windows_file_uri(mangled),
            "file:///E:/litecode/src/agent/core.rs"
        );
        let path = uri_to_path(mangled).expect("mangled uri should parse after normalize");
        assert_eq!(
            Some(path),
            uri_to_path("file:///E:/litecode/src/agent/core.rs"),
            "mangled and clean URIs map to the same path"
        );
        // The concrete `E:\` form is Windows path semantics.
        #[cfg(windows)]
        assert_eq!(
            uri_to_path(mangled).unwrap(),
            PathBuf::from(r"E:\litecode\src\agent\core.rs")
        );
    }

    #[test]
    fn coverage_requires_active_and_configured_command() {
        let hub = LspHub::new();
        let rs = Path::new("src/main.rs");
        assert!(
            !hub.file_has_lsp_coverage(rs),
            "inactive hub must not report coverage"
        );

        let cmd = server_map().get("rs").cloned().expect("rs mapping");
        hub.set_configured_commands_for_test(&[cmd]);
        assert!(hub.is_active());
        assert!(
            hub.file_has_lsp_coverage(rs),
            "configured rust-analyzer must cover .rs"
        );
        assert!(
            !hub.file_has_lsp_coverage(Path::new("readme.md")),
            "unmapped extension must not be covered"
        );

        // Only the configured command is honored — activate another ext's command
        // without rust-analyzer and .rs must lose coverage.
        let py = server_map().get("py").cloned().expect("py mapping");
        hub.set_configured_commands_for_test(&[py]);
        assert!(
            !hub.file_has_lsp_coverage(rs),
            "pyright-only config must not cover .rs"
        );
    }

    // ---- WP-167: deferred availability probe + sweep gate ----

    /// `sh <script>`: appends its argv to `log` on every run; `--version`
    /// exits with `version_exit`; any other invocation records `started` and
    /// lingers briefly like a server would. Run through `sh` so the test never
    /// execs a freshly written file (ETXTBSY under parallel tests).
    #[cfg(unix)]
    fn fake_ls(dir: &Path, version_exit: i32) -> (String, std::path::PathBuf) {
        let log = dir.join("calls.log");
        let script = dir.join("fake-ls.sh");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo version >> '{log}'; exit {version_exit}; fi\necho started >> '{log}'\nsleep 3\n",
                log = log.display()
            ),
        )
        .unwrap();
        (format!("sh {}", script.display()), log)
    }

    #[cfg(unix)]
    fn calls(log: &Path, what: &str) -> usize {
        std::fs::read_to_string(log)
            .unwrap_or_default()
            .lines()
            .filter(|line| *line == what)
            .count()
    }

    #[cfg(unix)]
    #[test]
    fn deferred_activate_runs_no_probe_until_first_need() {
        let dir = tempfile::tempdir().unwrap();
        let (cmd, log) = fake_ls(dir.path(), 0);
        let hub = LspHub::new();
        hub.set_workspace(dir.path().to_path_buf());
        hub.activate_deferred(&[cmd]).unwrap();
        assert!(hub.is_active());
        assert_eq!(
            calls(&log, "version"),
            0,
            "activation must not start a probe"
        );
        assert_eq!(hub.ensure_probed_blocking(), Ok(()));
        assert_eq!(calls(&log, "version"), 1);
    }

    #[cfg(unix)]
    #[test]
    fn probe_runs_once_per_activation() {
        let dir = tempfile::tempdir().unwrap();
        let (cmd, log) = fake_ls(dir.path(), 0);
        let hub = LspHub::new();
        hub.set_workspace(dir.path().to_path_buf());
        hub.activate_deferred(std::slice::from_ref(&cmd)).unwrap();
        assert_eq!(hub.ensure_probed_blocking(), Ok(()));
        assert_eq!(hub.ensure_probed_blocking(), Ok(()));
        assert_eq!(calls(&log, "version"), 1, "cached for this activation");
        hub.activate_deferred(&[cmd]).unwrap();
        assert_eq!(hub.ensure_probed_blocking(), Ok(()));
        assert_eq!(calls(&log, "version"), 2, "a new activation probes again");
    }

    #[cfg(unix)]
    #[test]
    fn probe_failure_deactivates_hub_and_reports_original_error() {
        let dir = tempfile::tempdir().unwrap();
        let (cmd, _log) = fake_ls(dir.path(), 3);
        let hub = LspHub::new();
        hub.set_workspace(dir.path().to_path_buf());
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let sink_seen = std::sync::Arc::clone(&seen);
        hub.set_probe_failure_sink(std::sync::Arc::new(move |e: String| {
            sink_seen.lock().unwrap().push(e);
        }));
        hub.activate_deferred(&[cmd]).unwrap();
        let err = hub.ensure_probed_blocking().expect_err("probe must fail");
        assert!(
            err.starts_with("language server 'sh' not runnable: "),
            "same text as the eager probe, got: {err}"
        );
        assert!(!hub.is_active(), "a failed probe leaves the hub inactive");
        assert_eq!(
            *seen.lock().unwrap(),
            vec![err.clone()],
            "sink gets the original error once"
        );
        let _ = hub.ensure_probed_blocking();
        assert_eq!(seen.lock().unwrap().len(), 1, "no second report");
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn restart_after_failed_probe_says_hub_not_active() {
        let dir = tempfile::tempdir().unwrap();
        let (cmd, log) = fake_ls(dir.path(), 3);
        let hub = std::sync::Arc::new(LspHub::new());
        hub.set_workspace(dir.path().to_path_buf());
        hub.activate_deferred(std::slice::from_ref(&cmd)).unwrap();
        let err = hub
            .restart_server(&cmd, dir.path())
            .await
            .expect_err("broken server");
        assert_eq!(
            err.to_string(),
            crate::types::LitecodeError::ToolExecution("LSP hub not active".into()).to_string()
        );
        assert_eq!(calls(&log, "started"), 0, "no spawn after a failed probe");
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn spawn_ls_waits_for_sweep_gate() {
        let dir = tempfile::tempdir().unwrap();
        let (cmd, log) = fake_ls(dir.path(), 0);
        let gate = crate::lsp::sweep_gate::SweepGate::running_for_test();
        let hub = std::sync::Arc::new(
            LspHub::new().with_sweep_gate_for_test(std::sync::Arc::clone(&gate)),
        );
        hub.set_workspace(dir.path().to_path_buf());
        hub.activate(std::slice::from_ref(&cmd)).unwrap();
        let task_hub = std::sync::Arc::clone(&hub);
        let root = dir.path().to_path_buf();
        let task_cmd = cmd.clone();
        let task = tokio::spawn(async move {
            let _ = task_hub.restart_server(&task_cmd, &root).await;
        });
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert_eq!(
            calls(&log, "started"),
            0,
            "no language server before the sweep finishes"
        );
        gate.open_for_test();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while calls(&log, "started") == 0 && std::time::Instant::now() < deadline {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert_eq!(
            calls(&log, "started"),
            1,
            "spawn proceeds once the gate opens"
        );
        task.abort();
        hub.stop().await;
    }
}

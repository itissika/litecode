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

    use serde_json::Value;

    use super::*;
    use crate::lsp::format::{extract_locations, format_error_diagnostics_block};
    use crate::lsp::hub::{normalize_lsp_action, short_feedback_reason};
    use crate::lsp::server::LspServer;
    use crate::lsp::server_map::default_server_map;
    use crate::lsp::uri::{
        normalize_windows_file_uri, publish_diagnostics_uri_matches, uri_to_path,
    };

    #[test]
    fn server_map_builtin_contains_rust_analyzer() {
        let map = default_server_map();
        assert_eq!(map.get("rs").map(String::as_str), Some("rust-analyzer"));
    }

    #[test]
    fn command_parts_preserves_quoted_program_path() {
        assert_eq!(
            command_parts("\"C:\\Program Files\\LLVM\\bin\\clangd.exe\" --background-index")
                .unwrap(),
            vec![
                "C:\\Program Files\\LLVM\\bin\\clangd.exe".to_string(),
                "--background-index".to_string(),
            ]
        );
    }

    #[test]
    fn detect_rust_from_cargo_toml() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"t\"\n").unwrap();
        let cmds = detect_needed_server_commands(dir.path());
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0], server_map().get("rs").cloned().unwrap());
    }

    #[test]
    fn extract_locations_accepts_location_and_location_link() {
        let location = serde_json::json!({
            "uri": "file:///e:/litecode/src/agent/deps.rs",
            "range": { "start": { "line": 4, "character": 10 }, "end": { "line": 4, "character": 19 } }
        });
        let link = serde_json::json!({
            "targetUri": "file:///e:/litecode/src/agent/deps.rs",
            "targetRange": { "start": { "line": 2, "character": 0 }, "end": { "line": 35, "character": 1 } },
            "targetSelectionRange": { "start": { "line": 4, "character": 10 }, "end": { "line": 4, "character": 19 } }
        });
        let from_loc = extract_locations(&serde_json::json!([location]));
        let from_link = extract_locations(&serde_json::json!([link]));
        assert_eq!(from_loc.len(), 1);
        assert_eq!(from_link.len(), 1);
        assert_eq!(from_loc[0].1, 5); // 0-based line 4 → 1-based 5
        assert_eq!(from_link[0].1, 3); // targetRange start line 2 → 3
        assert!(extract_locations(&Value::Null).is_empty());
        assert!(extract_locations(&serde_json::json!([])).is_empty());
    }

    #[test]
    fn configuration_response_roots_vs_nested() {
        let req = serde_json::json!({
            "params": {
                "items": [
                    { "section": "csharp" },
                    { "section": "csharp.solutionPathOverride" },
                    { "section": "rust-analyzer" },
                    { "section": "" },
                    {}
                ]
            }
        });
        let got = LspServer::configuration_response(&req);
        assert_eq!(got, serde_json::json!([{}, null, {}, null, null]));
    }

    #[test]
    fn error_diagnostics_block_silent_without_errors() {
        assert!(format_error_diagnostics_block(&serde_json::json!([])).is_none());
        assert!(
            format_error_diagnostics_block(&serde_json::json!([
                {"severity": 2, "message": "warn", "range": {"start": {"line": 0, "character": 0}}}
            ]))
            .is_none()
        );
    }

    #[test]
    fn error_diagnostics_block_only_includes_errors() {
        let text = format_error_diagnostics_block(&serde_json::json!([
            {"severity": 1, "message": "boom", "range": {"start": {"line": 2, "character": 4}}},
            {"severity": 2, "message": "warn", "range": {"start": {"line": 0, "character": 0}}}
        ]))
        .expect("errors present");
        assert!(text.contains("Error: boom"));
        assert!(!text.contains("warn"));
    }

    #[test]
    fn normalize_aliases() {
        assert_eq!(normalize_lsp_action("definition"), "goToDefinition");
        assert_eq!(normalize_lsp_action("references"), "findReferences");
        assert_eq!(
            normalize_lsp_action("goToImplementation"),
            "goToImplementation"
        );
    }

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

    #[test]
    fn strip_verbatim_drive_and_unc() {
        assert_eq!(
            crate::config::path::strip_verbatim(Path::new(r"\\?\E:\litecode")),
            PathBuf::from(r"E:\litecode")
        );
        assert_eq!(
            crate::config::path::strip_verbatim(Path::new(r"\\?\UNC\host\share\proj")),
            PathBuf::from(r"\\host\share\proj")
        );
        assert_eq!(
            crate::config::path::strip_verbatim(Path::new("/home/proj")),
            PathBuf::from("/home/proj")
        );
    }

    #[test]
    fn publish_diagnostics_uri_matches_drive_case() {
        assert!(publish_diagnostics_uri_matches(
            "file:///E:/litecode/src/a.rs",
            "file:///e:/litecode/src/a.rs"
        ));
        assert!(publish_diagnostics_uri_matches(
            "file:////?/E:/litecode/src/a.rs",
            "file:///E:/litecode/src/a.rs"
        ));
    }

    #[test]
    fn normalize_makes_verbatim_path_prefix_compatible() {
        let root = PathBuf::from(r"E:\litecode");
        let verbatim_file = PathBuf::from(r"\\?\E:\litecode\src\agent\core.rs");
        assert!(
            !verbatim_file.starts_with(&root),
            "Windows verbatim vs stripped root must diverge before normalize"
        );
        let normalized = crate::config::path::strip_verbatim(&verbatim_file);
        assert_eq!(normalized, PathBuf::from(r"E:\litecode\src\agent\core.rs"));
        assert!(
            normalized.starts_with(&root),
            "after normalize, hub starts_with must succeed"
        );
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
        let path = uri_to_path("file:////?/E:/litecode/src/agent/core.rs")
            .expect("mangled uri should parse after normalize");
        assert_eq!(path, PathBuf::from(r"E:\litecode\src\agent\core.rs"));
    }

    #[test]
    fn short_feedback_reason_truncates_long_errors() {
        let long = "x".repeat(200);
        let short = short_feedback_reason(&long);
        assert!(short.ends_with('…'));
        assert_eq!(short.chars().count(), 161); // 160 chars + ellipsis
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

    #[test]
    fn instance_statuses_empty_before_spawn() {
        let hub = LspHub::new();
        assert!(hub.instance_statuses().is_empty());
        hub.set_configured_commands_for_test(&["rust-analyzer".into()]);
        assert!(
            hub.instance_statuses().is_empty(),
            "activate/configure must not invent running instances"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn stop_clears_active_and_coverage() {
        let hub = std::sync::Arc::new(LspHub::new());
        let cmd = server_map().get("rs").cloned().unwrap();
        hub.set_configured_commands_for_test(&[cmd]);
        assert!(hub.file_has_lsp_coverage(Path::new("lib.rs")));
        hub.stop().await;
        assert!(!hub.is_active());
        assert!(!hub.file_has_lsp_coverage(Path::new("lib.rs")));
        assert!(hub.instance_statuses().is_empty());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn restart_server_rejects_inactive_hub() {
        let hub = std::sync::Arc::new(LspHub::new());
        let err = hub
            .restart_server("rust-analyzer", Path::new("/tmp/proj"))
            .await
            .expect_err("inactive hub");
        let msg = err.to_string();
        assert!(
            msg.contains("not active"),
            "expected inactive error, got: {msg}"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn restart_server_rejects_unconfigured_command() {
        let hub = std::sync::Arc::new(LspHub::new());
        hub.set_configured_commands_for_test(&["rust-analyzer".into()]);
        let err = hub
            .restart_server("gopls", Path::new("/tmp/proj"))
            .await
            .expect_err("unconfigured");
        let msg = err.to_string();
        assert!(
            msg.contains("not configured"),
            "expected not configured, got: {msg}"
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

    #[test]
    fn lifecycle_status_serializes_snake_case() {
        use crate::lsp::status::{LspInstanceStatus, LspLifecycle};
        let snap = LspInstanceStatus {
            command: "rust-analyzer".into(),
            project_root: "/proj".into(),
            state: LspLifecycle::Failed,
            index_settled: false,
            last_error: Some("closed stdout".into()),
            restart_count: 2,
        };
        let v = serde_json::to_value(&snap).unwrap();
        assert_eq!(v["state"], "failed");
        assert_eq!(v["index_settled"], false);
        assert_eq!(v["restart_count"], 2);
        assert_eq!(v["last_error"], "closed stdout");
    }
}

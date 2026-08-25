//! Bash tool for executing shell commands via TerminalHub.
//!
//! Agent shell I/O is a consumer of the process-scoped TerminalHub PTY.
//! Interactive human terminals use the same hub over the `terminal/*` WS API.

use serde_json::Value;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use crate::context_pipeline::Context;
use crate::terminal::{TerminalHub, WaitOutcome};
use crate::tool::Tool;
use crate::tool::trait_::ToolExecutionContext;
use crate::tool::write_lock::ResourceKey;
use crate::types::ToolCallResult;

use super::bash_safety::{is_destructive_command, is_readonly_command};
use super::bash_status;

pub const FOREGROUND_WAIT: Duration = Duration::from_secs(30);

pub struct BashTool {
    pub hub: Arc<TerminalHub>,
    /// Turn cancellation token captured from the execution context (REV-9:
    /// passed explicitly, never via TLS).
    cancel: CancellationToken,
    session_id: Mutex<String>,
    call_id: Mutex<String>,
    foreground_wait: Duration,
}

impl BashTool {
    pub fn new(hub: Arc<TerminalHub>) -> Self {
        Self {
            hub,
            cancel: CancellationToken::new(),
            session_id: Mutex::new(String::new()),
            call_id: Mutex::new(String::new()),
            foreground_wait: FOREGROUND_WAIT,
        }
    }

    pub(crate) fn with_foreground_wait(hub: Arc<TerminalHub>, foreground_wait: Duration) -> Self {
        Self {
            hub,
            cancel: CancellationToken::new(),
            session_id: Mutex::new(String::new()),
            call_id: Mutex::new(String::new()),
            foreground_wait,
        }
    }

    fn session_id(&self) -> String {
        self.session_id.lock().unwrap().clone()
    }

    fn call_id(&self) -> String {
        self.call_id.lock().unwrap().clone()
    }

    fn call_with_root(&self, input: Value, workspace_root: std::path::PathBuf) -> ToolCallResult {
        let command = match crate::tool::require_nonempty_string_trimmed(&input, "command") {
            Ok(c) => c,
            Err(e) => return ToolCallResult::error(e),
        };

        let run_in_background = input["run_in_background"].as_bool().unwrap_or(false);
        let workdir = input["workdir"].as_str().map(Path::new);
        let sid = self.session_id();
        let call_id = self.call_id();

        if run_in_background {
            return match self
                .hub
                .spawn_command(command, workdir, &workspace_root, &sid, &call_id)
            {
                Ok(spawned) => {
                    let jobs = self.hub.jobs.running(&sid);
                    ToolCallResult::ok(bash_status::format_running_status(
                        &spawned.id,
                        &spawned.output_path,
                        &workspace_root,
                        &jobs,
                    ))
                }
                Err(e) => ToolCallResult::error(e.to_string()),
            };
        }

        let spawned =
            match self
                .hub
                .spawn_command(command, workdir, &workspace_root, &sid, &call_id)
            {
                Ok(s) => s,
                Err(e) => return ToolCallResult::error(e.to_string()),
            };

        match self.hub.jobs.wait(
            &sid,
            Some(&spawned.id),
            Some(self.foreground_wait),
            &self.cancel,
            false,
        ) {
            WaitOutcome::Exited(notice) => {
                let capture = self
                    .hub
                    .jobs
                    .snapshot_capture(&notice.bash_id)
                    .unwrap_or_else(|| crate::terminal::TeeCapture {
                        path: notice.output_path.clone(),
                        head: String::new(),
                        tail: String::new(),
                        frozen: false,
                        total_bytes: 0,
                        truncated_on_disk: false,
                    });
                let body = bash_status::format_completed_view(
                    &capture,
                    notice.exit_code,
                    false,
                    &workspace_root,
                );
                ToolCallResult::ok(body)
            }
            WaitOutcome::TimedOut => {
                let jobs = self.hub.jobs.running(&sid);
                ToolCallResult::ok(bash_status::format_running_status(
                    &spawned.id,
                    &spawned.output_path,
                    &workspace_root,
                    &jobs,
                ))
            }
            WaitOutcome::Cancelled => {
                let _ = self.hub.kill(&spawned.id);
                let capture = self.hub.jobs.snapshot_capture(&spawned.id);
                let body = if let Some(cap) = capture {
                    bash_status::format_completed_view(&cap, None, true, &workspace_root)
                } else {
                    "status: cancelled\n".into()
                };
                ToolCallResult::error(body)
            }
            WaitOutcome::UnknownId(_) => ToolCallResult::error(format!(
                "background task '{}' not found after spawn",
                spawned.id
            )),
        }
    }
}

impl Tool for BashTool {
    fn name(&self) -> &str {
        "bash"
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "Shell command to run in one shot (not a persistent session). Chain with && when you need cd or env for this call only."
                },
                "workdir": {
                    "type": "string",
                    "description": "Working directory for this call only (workspace-relative preferred)."
                },
                "run_in_background": {
                    "type": "boolean",
                    "description": "Return immediately with bash_id. Inspect with read/grep; wait_shell to wait; kill_shell to stop.",
                    "default": false
                }
            },
            "required": ["command"]
        })
    }

    fn execute(
        &self,
        input: Value,
        execution: ToolExecutionContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>> {
        // REV-9: capture the turn cancellation token from the execution context
        // (never via TLS) so this tool can be cancelled across parallel handles.
        let tool = BashTool {
            hub: Arc::clone(&self.hub),
            cancel: execution.cancel.clone(),
            session_id: Mutex::new(execution.session_id.clone()),
            call_id: Mutex::new(execution.call_id.clone()),
            foreground_wait: self.foreground_wait,
        };
        let workspace_root = execution.workspace_root.clone();
        Box::pin(async move { tool.call_with_root(input, workspace_root) })
    }

    fn call_inner(&self, input: Value) -> ToolCallResult {
        self.call_with_root(input, crate::config::workspace::workspace_root_lap())
    }

    fn description(&self, _ctx: &Context) -> String {
        "Run a shell command. cwd and environment do not persist across calls — chain with && in one command. Prefer read, grep, and glob for files; use bash for builds, tests, git, and scripts. Short commands return when they exit. Longer ones keep running in the background: inspect with read/grep on the output file, wait_shell to wait, kill_shell to stop.".into()
    }

    fn max_result_size(&self) -> usize {
        usize::MAX
    }

    fn timeout(&self) -> Option<u64> {
        None
    }

    fn set_active_session(&self, session_id: String) {
        *self.session_id.lock().unwrap() = session_id;
    }

    fn agent_terminal(&self) -> Option<Arc<TerminalHub>> {
        Some(Arc::clone(&self.hub))
    }

    fn is_concurrency_safe(&self, input: &Value) -> bool {
        let command = match input["command"].as_str() {
            Some(cmd) => cmd,
            None => return false,
        };
        is_readonly_command(command)
    }

    fn is_cancellable(&self) -> bool {
        true
    }

    fn resource_keys(
        &self,
        input: &Value,
        path_mode: crate::workspace::ToolPathMode,
        workspace_root: &std::path::Path,
    ) -> Vec<ResourceKey> {
        // Per-path keys only, for same-turn partition (same-path read/write
        // serialize). Unparseable commands take no key: cross-session bash
        // races are accepted; mutating bash is already serial via
        // `is_concurrency_safe`.
        let command = match input["command"].as_str() {
            Some(c) => c,
            None => return vec![],
        };
        let paths = super::bash_safety::extract_bash_paths(command);
        if paths.is_empty() {
            return vec![];
        }
        let mut keys = Vec::new();
        for raw in paths {
            let key = crate::workspace::resolve_agent(workspace_root, &raw, path_mode)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| raw.clone());
            keys.push(ResourceKey::File(key));
        }
        keys
    }

    fn validate_input(&self, input: &Value) -> std::result::Result<(), String> {
        crate::tool::require_nonempty_string_trimmed(input, "command")?;

        if let Some(workdir) = input["workdir"].as_str() {
            let path = Path::new(workdir);
            if !path.exists() {
                return Err(format!("workdir does not exist: {workdir}"));
            }
        }

        Ok(())
    }

    fn is_destructive(
        &self,
        input: &Value,
        _path_mode: crate::workspace::ToolPathMode,
        _workspace_root: &std::path::Path,
    ) -> bool {
        let command = match input["command"].as_str() {
            Some(cmd) => cmd,
            None => return false,
        };
        is_destructive_command(command)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permission::PermissionAction;
    use crate::terminal::TeeCapture;
    use crate::tools::bash_safety::{check_dangerous_command, is_destructive_command};

    fn test_tool() -> BashTool {
        BashTool::new(Arc::new(TerminalHub::new()))
    }

    #[test]
    fn test_readonly_simple_commands() {
        assert!(is_readonly_command("ls"));
        assert!(is_readonly_command("ls -la /tmp"));
        assert!(is_readonly_command("cat file.txt"));
        assert!(is_readonly_command("pwd"));
        assert!(is_readonly_command("echo hello"));
        assert!(is_readonly_command("git status"));
        assert!(is_readonly_command("git log --oneline -5"));
        assert!(is_readonly_command("cargo check"));
        assert!(is_readonly_command("rustc --version"));
        assert!(is_readonly_command("ps aux"));
    }

    #[test]
    fn test_readonly_pipe_commands() {
        assert!(is_readonly_command("ls | head"));
        assert!(is_readonly_command("cat file.txt | grep foo | head -5"));
        assert!(is_readonly_command("git log | less"));
        assert!(is_readonly_command("find . -name '*.rs' | wc -l"));
    }

    #[test]
    fn test_not_readonly_commands() {
        assert!(!is_readonly_command("rm file.txt"));
        assert!(!is_readonly_command("apt install foo"));
        assert!(!is_readonly_command("echo hello > file.txt"));
        assert!(!is_readonly_command("make"));
        assert!(!is_readonly_command("cargo run"));
        assert!(!is_readonly_command("ls | sort | uniq > out.txt"));
    }

    #[test]
    fn test_readonly_empty_command() {
        assert!(!is_readonly_command(""));
        assert!(!is_readonly_command("  "));
    }

    #[test]
    fn test_deny_rm_rf_root() {
        assert_eq!(check_dangerous_command("rm -rf /"), PermissionAction::Deny);
        assert_eq!(check_dangerous_command("rm -rf /*"), PermissionAction::Deny);
        assert_eq!(check_dangerous_command("rm -fr /"), PermissionAction::Deny);
        assert_eq!(
            check_dangerous_command("sudo rm -rf /"),
            PermissionAction::Deny
        );
    }

    #[test]
    fn test_deny_fork_bomb() {
        assert_eq!(
            check_dangerous_command(":(){ :|:& };:"),
            PermissionAction::Deny
        );
    }

    #[test]
    fn test_deny_mkfs() {
        assert_eq!(
            check_dangerous_command("mkfs.ext4 /dev/sda1"),
            PermissionAction::Deny
        );
    }

    #[test]
    fn test_deny_dd_to_device() {
        assert_eq!(
            check_dangerous_command("dd if=/dev/zero of=/dev/sda"),
            PermissionAction::Deny
        );
    }

    #[test]
    fn test_deny_redirect_to_device() {
        assert_eq!(
            check_dangerous_command("cat foo > /dev/sda"),
            PermissionAction::Deny
        );
    }

    #[test]
    fn test_ask_rm_rf_specific() {
        assert_eq!(
            check_dangerous_command("rm -rf /tmp/build"),
            PermissionAction::Ask
        );
        assert_eq!(
            check_dangerous_command("rm -rf ./node_modules"),
            PermissionAction::Ask
        );
    }

    #[test]
    fn test_allow_safe_commands() {
        assert_eq!(check_dangerous_command("ls -la"), PermissionAction::Allow);
        assert_eq!(
            check_dangerous_command("cat file.txt"),
            PermissionAction::Allow
        );
        assert_eq!(
            check_dangerous_command("echo hello"),
            PermissionAction::Allow
        );
    }

    #[test]
    fn test_destructive_commands() {
        assert!(is_destructive_command("rm file.txt"));
        assert!(is_destructive_command("rmdir dir"));
        assert!(is_destructive_command("mv a b"));
        assert!(is_destructive_command("chmod 755 file"));
        assert!(is_destructive_command("chown user file"));
        assert!(is_destructive_command("kill 1234"));
        assert!(is_destructive_command("pkill firefox"));
        assert!(is_destructive_command("dd if=/dev/zero of=img.bin"));
    }

    #[test]
    fn test_not_destructive_commands() {
        assert!(!is_destructive_command("ls"));
        assert!(!is_destructive_command("cat file.txt"));
        assert!(!is_destructive_command("echo hello"));
        assert!(!is_destructive_command("git status"));
        assert!(!is_destructive_command("cp a b"));
    }

    #[test]
    fn test_validate_input_ok() {
        let input = serde_json::json!({"command": "ls"});
        assert!(test_tool().validate_input(&input).is_ok());
    }

    #[test]
    fn test_validate_input_missing_command() {
        let input = serde_json::json!({});
        assert!(test_tool().validate_input(&input).is_err());
    }

    #[test]
    fn test_validate_input_empty_command() {
        let input = serde_json::json!({"command": ""});
        assert!(test_tool().validate_input(&input).is_err());
    }

    #[test]
    fn test_validate_input_bad_workdir() {
        let input = serde_json::json!({"command": "ls", "workdir": "/no/such/path/ever"});
        assert!(test_tool().validate_input(&input).is_err());
    }

    fn sample_capture(
        frozen: bool,
        head: &str,
        tail: &str,
        total_bytes: usize,
        truncated_on_disk: bool,
    ) -> TeeCapture {
        TeeCapture {
            path: Path::new("/proj/.litecode/bash/bash_x.output").to_path_buf(),
            head: head.to_string(),
            tail: tail.to_string(),
            frozen,
            total_bytes,
            truncated_on_disk,
        }
    }

    #[test]
    fn small_success_puts_exit_code_first_and_omits_file() {
        let cap = sample_capture(false, "hello\n", "", 6, false);
        let body = bash_status::format_completed_view(&cap, Some(0), false, Path::new("/proj"));
        assert!(body.starts_with("exit_code: 0\n"));
        assert!(body.contains("hello"));
        assert!(!body.contains("output_file"));
    }

    #[test]
    fn large_success_shows_head_tail_and_file() {
        let cap = sample_capture(true, "HEADCHUNK\n", "TAILCHUNK\n", 50_000, false);
        let body = bash_status::format_completed_view(&cap, Some(1), false, Path::new("/proj"));
        assert_eq!(
            body,
            concat!(
                "exit_code: 1\n",
                "bytes: 50000\n",
                "output_file: .litecode/bash/bash_x.output\n",
                "[head 2048B + tail 4096B of 50000 bytes. output_file has the full log — grep/read it only if this window is not enough. Do not re-run.]\n",
                "\n",
                "--- head ---\n",
                "HEADCHUNK\n",
                "--- tail ---\n",
                "TAILCHUNK\n",
            )
        );
    }

    #[test]
    fn empty_success_is_exit_code_only() {
        let cap = sample_capture(false, "", "", 0, false);
        let body = bash_status::format_completed_view(&cap, Some(0), false, Path::new("/proj"));
        assert_eq!(body, "exit_code: 0\n");
    }

    #[test]
    fn cancelled_view_names_status() {
        let cap = sample_capture(false, "step 1\n", "", 7, false);
        let body = bash_status::format_completed_view(&cap, None, true, Path::new("/proj"));
        assert!(body.contains("status: cancelled"));
        assert!(body.contains("step 1"));
        assert!(!body.contains("output_file"));
    }

    #[test]
    fn truncated_on_disk_is_flagged_in_large_view() {
        let cap = sample_capture(true, "HEAD\n", "TAIL\n", 9_000_000, true);
        let body = bash_status::format_completed_view(&cap, Some(0), false, Path::new("/proj"));
        assert!(body.contains("truncated_on_disk: true"));
        assert!(body.contains("output_file:"));
    }

    #[test]
    fn display_path_strips_canonical_workspace_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let root = crate::config::path::canon_abs(dir.path()).unwrap();
        let path = root.join(".litecode").join("bash").join("bash_x.output");
        assert_eq!(
            bash_status::display_output_path(&path, &root),
            ".litecode/bash/bash_x.output"
        );
    }

    #[test]
    fn description_is_guidance_not_output_spec() {
        let ctx = crate::context_pipeline::Context {
            cwd: Path::new("/tmp").to_path_buf(),
            workspace_paths: crate::config::resolved::WorkspacePaths::for_legacy_root(Path::new(
                "/tmp",
            )),
            agents_md: None,
            claude_md: None,
        };
        let d = test_tool().description(&ctx);
        assert!(d.contains("do not persist"));
        assert!(d.contains("read"));
        assert!(d.contains("grep"));
        assert!(d.contains("wait_shell"));
        assert!(d.contains("kill_shell"));
        assert!(
            !d.contains("head+tail")
                && !d.contains(".litecode/bash")
                && !d.contains("do not re-run"),
            "output-window mechanics belong in the result, got: {d}"
        );
    }

    struct RuntimePathsGuard;
    impl Drop for RuntimePathsGuard {
        fn drop(&mut self) {
            crate::config::workspace::clear_runtime_paths();
        }
    }

    fn with_workspace(f: impl FnOnce(&Path, &BashTool)) {
        let dir = tempfile::tempdir().unwrap();
        let root = crate::config::path::canon_abs(dir.path()).unwrap();
        crate::config::workspace::set_runtime_paths(
            crate::config::resolved::WorkspacePaths::for_legacy_root(&root),
        );
        let _guard = RuntimePathsGuard;
        let tool = BashTool::new(Arc::new(TerminalHub::new()));
        tool.set_active_session("test".into());
        f(&root, &tool);
    }

    fn shell_cmd(unix: &str, powershell: &str) -> String {
        #[cfg(windows)]
        {
            if crate::config::git_install::find_git_bash().is_some() {
                unix.to_string()
            } else {
                powershell.to_string()
            }
        }
        #[cfg(not(windows))]
        {
            let _ = powershell;
            unix.to_string()
        }
    }

    fn bash_log_files(root: &Path) -> Vec<(std::path::PathBuf, String)> {
        let dir = root.join(".litecode").join("bash");
        let mut out = Vec::new();
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return out;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("output") {
                let body = std::fs::read_to_string(&path).unwrap_or_default();
                out.push((path, body));
            }
        }
        out
    }

    #[test]
    fn live_small_echo_exit_code_first_file_on_disk_not_in_view() {
        with_workspace(|root, tool| {
            let cmd = shell_cmd("echo LITECODE_SMALL_OK", "Write-Output 'LITECODE_SMALL_OK'");
            let result =
                tool.call_with_root(serde_json::json!({ "command": cmd }), root.to_path_buf());
            assert_eq!(result.level, crate::types::ToolSignalLevel::Ok);
            assert!(
                result.content.starts_with("exit_code: 0\n"),
                "got: {}",
                result.content
            );
            assert!(
                result.content.contains("LITECODE_SMALL_OK"),
                "got: {}",
                result.content
            );
            assert!(
                !result.content.contains("output_file"),
                "small view must not point at the log, got: {}",
                result.content
            );
            let logs = bash_log_files(root);
            assert_eq!(logs.len(), 1, "expected one tee file, got {logs:?}");
            assert!(
                logs[0].1.contains("LITECODE_SMALL_OK"),
                "disk log missing marker: {}",
                logs[0].1
            );
        });
    }

    #[test]
    fn live_nonzero_exit_is_ok_with_code() {
        with_workspace(|root, tool| {
            let cmd = shell_cmd("exit 7", "exit 7");
            let result =
                tool.call_with_root(serde_json::json!({ "command": cmd }), root.to_path_buf());
            assert_eq!(result.level, crate::types::ToolSignalLevel::Ok);
            assert!(
                result.content.starts_with("exit_code: 7\n"),
                "got: {}",
                result.content
            );
        });
    }

    #[test]
    fn live_large_output_windows_and_keeps_full_log() {
        with_workspace(|root, tool| {
            let cmd = shell_cmd(
                "i=1; while [ $i -le 2500 ]; do echo LINE_$i; i=$((i+1)); done",
                "1..2500 | ForEach-Object { \"LINE_$_\" }",
            );
            let result =
                tool.call_with_root(serde_json::json!({ "command": cmd }), root.to_path_buf());
            assert_eq!(result.level, crate::types::ToolSignalLevel::Ok);
            assert!(
                result.content.contains("output_file: .litecode/bash/"),
                "expected workspace-relative log path, got: {}",
                result.content
            );
            assert!(result.content.contains("--- head ---"));
            assert!(result.content.contains("--- tail ---"));
            assert!(result.content.contains("only if this window is not enough"));
            assert!(
                result.content.contains("LINE_1"),
                "head should include early lines, got: {}",
                result.content
            );
            assert!(
                result.content.contains("LINE_2500"),
                "tail should include last lines, got: {}",
                result.content
            );
            let logs = bash_log_files(root);
            assert_eq!(logs.len(), 1);
            assert!(
                logs[0].1.contains("LINE_1200"),
                "full log should keep the middle; file len {}",
                logs[0].1.len()
            );
            assert!(
                !result.content.contains("LINE_1200"),
                "middle of a large log must not fill the window"
            );
        });
    }

    #[test]
    fn live_detach_keeps_process_and_log() {
        with_workspace(|root, tool| {
            let tool = BashTool {
                hub: Arc::clone(&tool.hub),
                cancel: CancellationToken::new(),
                session_id: Mutex::new("test".into()),
                call_id: Mutex::new("call_live".into()),
                foreground_wait: Duration::from_millis(400),
            };
            let cmd = shell_cmd(
                "echo LITECODE_TIMEOUT_MARK; sleep 8",
                "Write-Output 'LITECODE_TIMEOUT_MARK'; Start-Sleep -Seconds 8",
            );
            let result =
                tool.call_with_root(serde_json::json!({ "command": cmd }), root.to_path_buf());
            assert_eq!(result.level, crate::types::ToolSignalLevel::Ok);
            assert!(
                result.content.contains("status: running"),
                "got: {}",
                result.content
            );
            assert!(result.content.contains("bash_id:"));
            assert!(result.content.contains("wait_shell"));
            let id = result
                .content
                .lines()
                .find_map(|l| l.strip_prefix("bash_id: "))
                .expect("id");
            assert!(
                tool.hub.jobs.get(id).is_some_and(|(alive, _, _, _)| alive),
                "process should still be running"
            );
            let logs = bash_log_files(root);
            assert_eq!(logs.len(), 1);
            let started = std::time::Instant::now();
            let mut body = String::new();
            while started.elapsed() < std::time::Duration::from_secs(3) {
                body = std::fs::read_to_string(&logs[0].0).unwrap_or_default();
                if body.contains("LITECODE_TIMEOUT_MARK") {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            assert!(body.contains("LITECODE_TIMEOUT_MARK"), "got: {body}");
            let _ = tool.hub.kill(id);
            let _ = tool.hub.close_agent(id);
        });
    }

    #[test]
    fn live_background_returns_id_and_relative_file() {
        with_workspace(|root, tool| {
            let cmd = shell_cmd(
                "echo LITECODE_BG_MARK; sleep 20",
                "Write-Output 'LITECODE_BG_MARK'; Start-Sleep -Seconds 20",
            );
            let result = tool.call_with_root(
                serde_json::json!({ "command": cmd, "run_in_background": true }),
                root.to_path_buf(),
            );
            assert_eq!(result.level, crate::types::ToolSignalLevel::Ok);
            assert!(
                result.content.contains("bash_id:"),
                "got: {}",
                result.content
            );
            assert!(
                result.content.contains("output_file: .litecode/bash/"),
                "got: {}",
                result.content
            );
            assert!(result.content.contains("kill_shell"));
            let id = result
                .content
                .lines()
                .find_map(|l| l.strip_prefix("bash_id: "))
                .expect("id");
            assert!(
                id.starts_with("bg_")
                    && id.len() == 11
                    && id[3..].chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')),
                "expected short bash_id, got {id}"
            );
            let log = root
                .join(".litecode")
                .join("bash")
                .join(format!("{id}.output"));
            let started = std::time::Instant::now();
            let mut body = String::new();
            while started.elapsed() < std::time::Duration::from_secs(3) {
                body = std::fs::read_to_string(&log).unwrap_or_default();
                if body.contains("LITECODE_BG_MARK") {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            assert!(
                body.contains("LITECODE_BG_MARK"),
                "background log missing marker at {}: {body}",
                log.display()
            );
            let _ = tool.hub.kill(id);
            let _ = tool.hub.close_agent(id);
        });
    }

    // bash's `resource_keys` returns per-path keys for same-turn partition.
    // Unparseable commands return no keys: they do not take a workspace lock.
    #[test]
    fn resource_keys_are_per_path_for_readonly_and_write_commands() {
        let tool = test_tool();
        let root = std::path::Path::new("/proj");
        let mode = crate::workspace::ToolPathMode::All;

        // Same raw path from a read (`cat`) and a write (`rm`) must resolve to
        // the SAME File key → they serialize against each other. (Exact path
        // spelling varies by platform; the important invariant is key equality.)
        let read = tool.resource_keys(&serde_json::json!({"command": "cat src/a.rs"}), mode, root);
        let write = tool.resource_keys(&serde_json::json!({"command": "rm src/a.rs"}), mode, root);
        let read_file = read.iter().find_map(|k| match k {
            ResourceKey::File(p) => Some(p.clone()),
            _ => None,
        });
        let write_file = write.iter().find_map(|k| match k {
            ResourceKey::File(p) => Some(p.clone()),
            _ => None,
        });
        assert!(read_file.is_some(), "read must yield a File key: {read:?}");
        assert_eq!(
            read_file.as_deref(),
            write_file.as_deref(),
            "same path read/write must share the File key"
        );
        assert!(
            !read.contains(&ResourceKey::Workspace),
            "read-only bash must not take the coarse workspace lock: {read:?}"
        );
    }

    #[test]
    fn resource_keys_do_not_use_workspace_coarse_lock_for_path_commands() {
        let tool = test_tool();
        let root = std::path::Path::new("/proj");
        let mode = crate::workspace::ToolPathMode::All;

        // `rm` on a path returns a per-path File key, not Workspace.
        let keys = tool.resource_keys(
            &serde_json::json!({"command": "rm /proj/build"}),
            mode,
            root,
        );
        assert!(
            keys.iter().any(|k| matches!(k, ResourceKey::File(_))),
            "expected a per-path File key, got: {keys:?}"
        );
        assert!(
            !keys.contains(&ResourceKey::Workspace),
            "path command must not take the coarse workspace lock: {keys:?}"
        );
    }

    #[test]
    fn resource_keys_are_empty_for_unparseable_command() {
        let tool = test_tool();
        let root = std::path::Path::new("/proj");
        let mode = crate::workspace::ToolPathMode::All;

        let keys = tool.resource_keys(&serde_json::json!({"command": "make"}), mode, root);
        assert!(
            keys.is_empty(),
            "unparseable bash must not take a lock: {keys:?}"
        );

        let keys = tool.resource_keys(&serde_json::json!({}), mode, root);
        assert!(keys.is_empty());
    }
}

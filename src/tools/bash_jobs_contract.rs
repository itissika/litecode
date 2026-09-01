//! Typical bash / wait_shell / kill_shell flows: result text must equal production formatters.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::terminal::TerminalHub;
use crate::tool::Tool;
use crate::tool::trait_::ToolExecutionContext;
use crate::tools::bash::BashTool;
use crate::tools::bash_status;
use crate::tools::kill_shell::KillShellTool;
use crate::tools::wait_shell::WaitShellTool;
use crate::types::{ToolCallResult, ToolSignalLevel};

struct Flow {
    _dir: tempfile::TempDir,
    root: PathBuf,
    hub: Arc<TerminalHub>,
    sid: String,
}

impl Flow {
    fn new(sid: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = crate::config::path::canon_abs(dir.path()).unwrap();
        Self {
            _dir: dir,
            root,
            hub: Arc::new(TerminalHub::new()),
            sid: sid.into(),
        }
    }

    fn bash(&self, wait: Option<Duration>) -> BashTool {
        match wait {
            Some(d) => BashTool::with_foreground_wait(Arc::clone(&self.hub), d),
            None => BashTool::new(Arc::clone(&self.hub)),
        }
    }

    fn wait_tool(&self) -> WaitShellTool {
        let t = WaitShellTool::new(Arc::clone(&self.hub));
        t.set_active_session(self.sid.clone());
        t
    }

    fn kill_tool(&self) -> KillShellTool {
        let t = KillShellTool::new(Arc::clone(&self.hub));
        t.set_active_session(self.sid.clone());
        t
    }

    fn exec(tool: &dyn Tool, input: Value, root: &Path, sid: &str) -> ToolCallResult {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(tool.execute(
            input,
            ToolExecutionContext {
                path_mode: crate::workspace::ToolPathMode::All,
                workspace_root: root.to_path_buf(),
                call_id: "flow".into(),
                cancel: CancellationToken::new(),
                output_limit: usize::MAX,
                session_id: sid.into(),
                session: None,
            },
        ))
        .finalize_signals()
    }

    fn run_bash(&self, input: Value, wait: Option<Duration>) -> ToolCallResult {
        Self::exec(&self.bash(wait), input, &self.root, &self.sid)
    }

    fn run_wait(&self, input: Value) -> ToolCallResult {
        Self::exec(&self.wait_tool(), input, &self.root, &self.sid)
    }

    fn run_kill(&self, bash_id: &str) -> ToolCallResult {
        Self::exec(
            &self.kill_tool(),
            serde_json::json!({ "bash_id": bash_id }),
            &self.root,
            &self.sid,
        )
    }
}

impl Drop for Flow {
    fn drop(&mut self) {
        for job in self.hub.jobs.running(&self.sid) {
            let _ = self.hub.kill(&job.id);
            let _ = self.hub.close_agent(&job.id);
        }
    }
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

fn sleep_cmd(secs: u64) -> String {
    shell_cmd(
        &format!("sleep {secs}"),
        &format!("Start-Sleep -Seconds {secs}"),
    )
}

fn echo_cmd(mark: &str) -> String {
    shell_cmd(&format!("echo {mark}"), &format!("Write-Output '{mark}'"))
}

fn bash_id_of(content: &str) -> String {
    content
        .lines()
        .find_map(|l| l.strip_prefix("bash_id: "))
        .unwrap_or_else(|| panic!("missing bash_id in:\n{content}"))
        .to_string()
}

fn wait_until(hub: &TerminalHub, id: &str, alive: bool) {
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(12) {
        if hub.jobs.get(id).is_some_and(|(a, _, _, _)| a == alive) {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("job {id} did not reach alive={alive}");
}

fn expected_running(flow: &Flow, id: &str) -> String {
    let notice = flow.hub.jobs.notice_snapshot(id).expect("job");
    let jobs = flow.hub.jobs.running(&flow.sid);
    bash_status::format_running_status(&id, &notice.output_path, &flow.root, &jobs)
}

fn expected_exited(flow: &Flow, id: &str) -> String {
    let notice = flow.hub.jobs.notice_snapshot(id).expect("job");
    let jobs = flow.hub.jobs.running(&flow.sid);
    bash_status::format_exited_status(&notice, &flow.root, &jobs)
}

fn expected_waited(flow: &Flow) -> String {
    bash_status::format_waited_status(&flow.hub.jobs.running(&flow.sid), &flow.root)
}

#[test]
fn background_launch_feedback_equals_formatter() {
    let flow = Flow::new("s1");
    let cmd = format!("{}; {}", echo_cmd("BG_MARK"), sleep_cmd(20));
    let result = flow.run_bash(
        serde_json::json!({ "command": cmd, "run_in_background": true }),
        None,
    );
    assert_eq!(result.level, ToolSignalLevel::Ok);
    let id = bash_id_of(&result.content);
    assert_eq!(result.content, expected_running(&flow, &id));
}

#[test]
fn foreground_detach_feedback_equals_formatter_and_keeps_process() {
    let flow = Flow::new("s1");
    let cmd = format!("{}; {}", echo_cmd("DETACH_MARK"), sleep_cmd(20));
    let result = flow.run_bash(
        serde_json::json!({ "command": cmd }),
        Some(Duration::from_millis(400)),
    );
    assert_eq!(result.level, ToolSignalLevel::Ok);
    let id = bash_id_of(&result.content);
    assert_eq!(result.content, expected_running(&flow, &id));
    assert!(flow.hub.jobs.get(&id).is_some_and(|(alive, _, _, _)| alive));
}

#[test]
fn short_command_uses_completed_view_not_running_status() {
    let flow = Flow::new("s1");
    let result = flow.run_bash(serde_json::json!({ "command": echo_cmd("SHORT_OK") }), None);
    assert_eq!(result.level, ToolSignalLevel::Ok);
    assert!(
        result.content.starts_with("exit_code: 0\n"),
        "{}",
        result.content
    );
    assert!(!result.content.contains("status: running"));
    assert!(!result.content.contains("output_file"));
    assert!(result.content.contains("SHORT_OK"), "{}", result.content);
}

#[test]
fn wait_id_after_exit_equals_exited_formatter() {
    let flow = Flow::new("s1");
    let launched = flow.run_bash(
        serde_json::json!({ "command": echo_cmd("WAIT_DONE"), "run_in_background": true }),
        None,
    );
    let id = bash_id_of(&launched.content);
    wait_until(&flow.hub, &id, false);
    let result = flow.run_wait(serde_json::json!({ "id": id }));
    assert_eq!(result.level, ToolSignalLevel::Ok);
    assert_eq!(result.content, expected_exited(&flow, &id));
    assert!(!result.content.contains("<system-reminder>"));
}

#[test]
fn wait_sec_only_equals_waited_formatter_and_does_not_kill() {
    let flow = Flow::new("s1");
    let launched = flow.run_bash(
        serde_json::json!({ "command": sleep_cmd(20), "run_in_background": true }),
        None,
    );
    let id = bash_id_of(&launched.content);
    let started = Instant::now();
    let result = flow.run_wait(serde_json::json!({ "sec": 1 }));
    assert!(started.elapsed() < Duration::from_secs(4));
    assert_eq!(result.level, ToolSignalLevel::Ok);
    assert_eq!(result.content, expected_waited(&flow));
    assert!(flow.hub.jobs.get(&id).is_some_and(|(alive, _, _, _)| alive));
}

#[test]
fn wait_id_and_sec_timer_wins_while_job_runs() {
    let flow = Flow::new("s1");
    let launched = flow.run_bash(
        serde_json::json!({ "command": sleep_cmd(20), "run_in_background": true }),
        None,
    );
    let id = bash_id_of(&launched.content);
    let result = flow.run_wait(serde_json::json!({ "id": id, "sec": 1 }));
    assert_eq!(result.level, ToolSignalLevel::Ok);
    assert_eq!(result.content, expected_waited(&flow));
    assert!(flow.hub.jobs.get(&id).is_some_and(|(alive, _, _, _)| alive));
}

#[test]
fn wait_id_and_sec_exit_wins_before_timer() {
    let flow = Flow::new("s1");
    let launched = flow.run_bash(
        serde_json::json!({ "command": echo_cmd("FAST"), "run_in_background": true }),
        None,
    );
    let id = bash_id_of(&launched.content);
    wait_until(&flow.hub, &id, false);
    let result = flow.run_wait(serde_json::json!({ "id": id, "sec": 30 }));
    assert_eq!(result.level, ToolSignalLevel::Ok);
    assert_eq!(result.content, expected_exited(&flow, &id));
}

#[test]
fn wait_unknown_id_equals_unknown_formatter_with_running_list() {
    let flow = Flow::new("s1");
    let launched = flow.run_bash(
        serde_json::json!({ "command": sleep_cmd(20), "run_in_background": true }),
        None,
    );
    let live = bash_id_of(&launched.content);
    let result = flow.run_wait(serde_json::json!({ "id": "bg_missing" }));
    assert_eq!(result.level, ToolSignalLevel::Error);
    assert_eq!(
        result.content,
        format!(
            "Error: {}",
            bash_status::format_unknown_task(
                "bg_missing",
                &flow.hub.jobs.running(&flow.sid),
                &flow.root
            )
        )
    );
    assert!(result.content.contains(&live));
}

#[test]
fn wait_wakes_on_other_session_job_exit() {
    let flow = Flow::new("s1");
    let long = flow.run_bash(
        serde_json::json!({ "command": sleep_cmd(20), "run_in_background": true }),
        None,
    );
    let long_id = bash_id_of(&long.content);
    let quick = flow.run_bash(
        serde_json::json!({ "command": echo_cmd("OTHER"), "run_in_background": true }),
        None,
    );
    let quick_id = bash_id_of(&quick.content);
    let result = flow.run_wait(serde_json::json!({ "id": long_id, "sec": 15 }));
    assert_eq!(result.level, ToolSignalLevel::Ok);
    assert_eq!(result.content, expected_exited(&flow, &quick_id));
    assert!(
        flow.hub
            .jobs
            .get(&long_id)
            .is_some_and(|(alive, _, _, _)| alive),
        "wait must not kill the watched job"
    );
}

#[test]
fn foreground_wait_ignores_other_job_exit() {
    let flow = Flow::new("s1");
    let other = flow.run_bash(
        serde_json::json!({ "command": echo_cmd("OTHER_EXIT"), "run_in_background": true }),
        None,
    );
    let other_id = bash_id_of(&other.content);
    let result = flow.run_bash(
        serde_json::json!({ "command": sleep_cmd(8) }),
        Some(Duration::from_millis(800)),
    );
    assert_eq!(result.level, ToolSignalLevel::Ok);
    let id = bash_id_of(&result.content);
    assert_ne!(id, other_id);
    assert_eq!(result.content, expected_running(&flow, &id));
    assert!(
        !result.content.starts_with("exit_code:"),
        "other job exit must not look like this command finished: {}",
        result.content
    );
}

#[test]
fn kill_feedback_equals_formatter_and_lists_remaining() {
    let flow = Flow::new("s1");
    let a = flow.run_bash(
        serde_json::json!({ "command": sleep_cmd(20), "run_in_background": true }),
        None,
    );
    let b = flow.run_bash(
        serde_json::json!({ "command": sleep_cmd(20), "run_in_background": true }),
        None,
    );
    let a_id = bash_id_of(&a.content);
    let b_id = bash_id_of(&b.content);
    let result = flow.run_kill(&a_id);
    assert_eq!(result.level, ToolSignalLevel::Ok);
    let notice = flow.hub.jobs.notice_snapshot(&a_id).expect("killed job");
    let expected = bash_status::format_killed_status(
        &a_id,
        notice.exit_code.map(|c| c as i32).unwrap_or(-1),
        Some(&notice.output_path),
        &flow.root,
        &flow.hub.jobs.running(&flow.sid),
    );
    assert_eq!(result.content, expected);
    assert!(result.content.contains(&b_id));
    assert!(!result.content.contains(&format!("- {a_id}  ")));
}

#[test]
fn wait_after_kill_returns_exited_not_timeout() {
    let flow = Flow::new("s1");
    let launched = flow.run_bash(
        serde_json::json!({ "command": sleep_cmd(20), "run_in_background": true }),
        None,
    );
    let id = bash_id_of(&launched.content);
    let killed = flow.run_kill(&id);
    assert_eq!(killed.level, ToolSignalLevel::Ok);
    assert!(
        !flow.hub.jobs.get(&id).is_some_and(|(alive, _, _, _)| alive),
        "killed job must leave the running set"
    );
    let started = Instant::now();
    let waited = flow.run_wait(serde_json::json!({ "id": id, "sec": 8 }));
    assert!(started.elapsed() < Duration::from_secs(3));
    assert_eq!(waited.level, ToolSignalLevel::Ok);
    assert!(
        waited.content.contains("status: exited"),
        "got: {}",
        waited.content
    );
    assert!(!waited.content.contains("status: waited"));
}

#[test]
fn wait_after_ui_kill_tells_agent_user_stopped() {
    let flow = Flow::new("s1");
    let launched = flow.run_bash(
        serde_json::json!({ "command": sleep_cmd(20), "run_in_background": true }),
        None,
    );
    let id = bash_id_of(&launched.content);
    flow.hub.kill_from_ui(&id).expect("ui kill");
    let _ = flow.hub.close_agent(&id);
    let waited = flow.run_wait(serde_json::json!({ "id": id, "sec": 8 }));
    assert_eq!(waited.level, ToolSignalLevel::Ok);
    assert!(
        waited.content.contains("stopped_by: user (Kill)"),
        "got: {}",
        waited.content
    );
}

#[test]
fn kill_unknown_equals_unknown_formatter() {
    let flow = Flow::new("s1");
    let result = flow.run_kill("bg_nope");
    assert_eq!(result.level, ToolSignalLevel::Error);
    assert_eq!(
        result.content,
        format!(
            "Error: {}",
            bash_status::format_unknown_task("bg_nope", &[], &flow.root)
        )
    );
}

#[test]
fn running_list_is_session_scoped() {
    let flow = Flow::new("s1");
    let other_sid = "s2";
    let mine = flow.run_bash(
        serde_json::json!({ "command": sleep_cmd(20), "run_in_background": true }),
        None,
    );
    let mine_id = bash_id_of(&mine.content);
    let other = flow
        .hub
        .spawn_command(&sleep_cmd(20), None, &flow.root, other_sid, "")
        .unwrap();
    let listed = flow.hub.jobs.running(&flow.sid);
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, mine_id);
    assert!(!listed.iter().any(|j| j.id == other.id));
    let wait = flow.run_wait(serde_json::json!({ "id": other.id }));
    assert_eq!(wait.level, ToolSignalLevel::Error);
    assert_eq!(
        wait.content,
        format!(
            "Error: {}",
            bash_status::format_unknown_task(
                &other.id,
                &flow.hub.jobs.running(&flow.sid),
                &flow.root
            )
        )
    );
    let _ = flow.hub.kill(&other.id);
    let _ = flow.hub.close_agent(&other.id);
}

#[test]
fn wait_validate_copy_matches_schema_helpers() {
    let tool = WaitShellTool::new(Arc::new(TerminalHub::new()));
    assert_eq!(
        tool.validate_input(&serde_json::json!({})).unwrap_err(),
        "missing required parameter 'id' or 'sec'"
    );
    assert_eq!(
        tool.validate_input(&serde_json::json!({"sec": 0}))
            .unwrap_err(),
        crate::tool::must_be("sec", "between 1 and 600")
    );
    assert_eq!(
        tool.validate_input(&serde_json::json!({"sec": 1.5}))
            .unwrap_err(),
        crate::tool::expected_type("sec", "integer", &serde_json::json!(1.5))
    );
    assert!(tool.validate_input(&serde_json::json!({"id": ""})).is_err());
}

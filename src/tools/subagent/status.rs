//! Agent-facing subagent job status text (running list + reminders).
//!
//! Stored facts are raw `turn/end.reason` values. Labels here are the agent
//! client's rendering of those facts (same mapping humans get from `turn_error`).

pub use super::jobs::{format_exit_reminder, format_running_list};
use super::jobs::{ExitNotice, RunningJob};

pub fn guidance_line() -> &'static str {
    "Use subagent_list to list sessions. subagent_wait to wait. subagent_stop to cancel the current turn. session_search to read a child's transcript."
}

fn reason_output(notice: &ExitNotice) -> String {
    match notice.reason.as_str() {
        "max_steps" => {
            const MSG: &str = "max steps reached";
            if notice.final_text.is_empty() {
                MSG.to_string()
            } else if notice.final_text.contains(MSG) {
                notice.final_text.clone()
            } else {
                format!("{}\n{MSG}", notice.final_text)
            }
        }
        _ => notice.final_text.clone(),
    }
}

fn append_output(out: &mut String, notice: &ExitNotice) {
    let text = reason_output(notice);
    if text.is_empty() {
        return;
    }
    out.push_str("output:\n");
    out.push_str(&text);
    if !text.ends_with('\n') {
        out.push('\n');
    }
}

pub fn format_running_status(child_id: &str, jobs: &[RunningJob]) -> String {
    let mut out = String::new();
    out.push_str("status: running\n");
    out.push_str(&format!("child_session_id: {child_id}\n"));
    out.push_str(&format_running_list(jobs));
    out.push_str(guidance_line());
    out.push('\n');
    out
}

pub fn format_sent_status(child_id: &str, turn_id: &str, jobs: &[RunningJob]) -> String {
    let mut out = String::new();
    out.push_str("status: running\n");
    out.push_str(&format!("child_session_id: {child_id}\n"));
    out.push_str(&format!("turn_id: {turn_id}\n"));
    out.push_str(&format_running_list(jobs));
    out.push_str(guidance_line());
    out.push('\n');
    out
}

pub fn format_completed_status(notice: &ExitNotice, jobs: &[RunningJob]) -> String {
    let mut out = String::new();
    match notice.reason.as_str() {
        "cancelled" => out.push_str("status: cancelled\n"),
        "completed" => out.push_str("status: completed\n"),
        "unknown" => out.push_str("status: unknown\n"),
        _ => out.push_str("status: failed\n"),
    }
    out.push_str(&format!("child_session_id: {}\n", notice.child_session_id));
    out.push_str(&format!("agent: {}\n", notice.agent_name));
    append_output(&mut out, notice);
    out.push_str(&format_running_list(jobs));
    out.push_str(guidance_line());
    out.push('\n');
    out
}

pub fn format_exited_status(notice: &ExitNotice, jobs: &[RunningJob]) -> String {
    let mut out = String::new();
    out.push_str("status: exited\n");
    out.push_str(&format!("child_session_id: {}\n", notice.child_session_id));
    out.push_str(&format!("agent: {}\n", notice.agent_name));
    if notice.stopped {
        out.push_str("stopped: true\n");
    }
    if notice.reason == "unknown" {
        out.push_str("reason: unknown\n");
    } else if !notice.ok && !notice.stopped {
        out.push_str("ok: false\n");
    }
    if !notice.prompt_preview.is_empty() {
        out.push_str(&format!("task: {}\n", notice.prompt_preview));
    }
    append_output(&mut out, notice);
    out.push_str(&format_running_list(jobs));
    out.push_str(guidance_line());
    out.push('\n');
    out
}

pub fn format_waited_status(jobs: &[RunningJob]) -> String {
    let mut out = String::from("status: still running\n");
    out.push_str(&format_running_list(jobs));
    out.push_str(guidance_line());
    out.push('\n');
    out
}

pub fn format_stopping_status(child_id: &str, jobs: &[RunningJob]) -> String {
    let mut msg = format!("status: stopping\nchild_session_id: {child_id}\n");
    msg.push_str(&format_running_list(jobs));
    msg.push_str(guidance_line());
    msg.push('\n');
    msg
}

pub fn format_already_ended_status(notice: &ExitNotice, jobs: &[RunningJob]) -> String {
    let mut out = String::from("status: already ended\n");
    out.push_str(&format!("child_session_id: {}\n", notice.child_session_id));
    out.push_str(&format!("agent: {}\n", notice.agent_name));
    let outcome = match notice.reason.as_str() {
        "cancelled" => "cancelled",
        "completed" => "completed",
        "unknown" => "unknown",
        _ => "failed",
    };
    out.push_str(&format!("outcome: {outcome}\n"));
    if !notice.prompt_preview.is_empty() {
        out.push_str(&format!("task: {}\n", notice.prompt_preview));
    }
    if outcome == "failed" {
        let text = reason_output(notice);
        if !text.is_empty() {
            let reason: String = text.chars().take(200).collect();
            out.push_str(&format!("reason: {reason}\n"));
        }
    }
    out.push_str("hint: use subagent_wait to fetch the result.\n");
    out.push_str(&format_running_list(jobs));
    out.push_str(guidance_line());
    out.push('\n');
    out
}

pub fn format_unknown_task(child_id: &str, jobs: &[RunningJob]) -> String {
    let mut msg = format!(
        "subagent '{child_id}' not found. It may have already exited or never existed.\n"
    );
    msg.push_str(&format_running_list(jobs));
    msg
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now_ms() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }

    fn notice(reason: &str, text: &str) -> ExitNotice {
        ExitNotice {
            child_session_id: "child-a".into(),
            parent_session_id: "p1".into(),
            agent_name: "reviewer".into(),
            prompt_preview: "review this".into(),
            reason: reason.into(),
            ok: reason == "completed",
            stopped: reason == "cancelled",
            final_text: text.into(),
        }
    }

    #[test]
    fn running_status_includes_id_and_guidance() {
        let jobs = vec![RunningJob {
            id: "child-a".into(),
            agent_name: "reviewer".into(),
            prompt_preview: "review this".into(),
            started_at_ms: now_ms(),
        }];
        let got = format_running_status("child-a", &jobs);
        assert!(got.starts_with("status: running\nchild_session_id: child-a\n"));
        assert!(got.contains("- child-a  reviewer  "), "got: {got}");
        assert!(got.contains("  review this\n"), "got: {got}");
        assert!(got.contains(guidance_line()));
        assert!(got.contains("running: 1\n"), "got: {got}");
        assert!(!got.contains("running: 1/"), "got: {got}");
    }

    #[test]
    fn max_steps_renders_human_reason() {
        let got = format_exited_status(&notice("max_steps", ""), &[]);
        assert!(got.contains("ok: false"), "{got}");
        assert!(got.contains("max steps reached"), "{got}");
        assert!(!got.contains("stopped: true"), "{got}");
    }

    #[test]
    fn unknown_does_not_render_as_failed() {
        let got = format_exited_status(&notice("unknown", ""), &[]);
        assert!(got.contains("reason: unknown"), "{got}");
        assert!(!got.contains("ok: false"), "{got}");
        assert!(!got.contains("status: failed"), "{got}");
    }
}

//! Agent-facing subagent job status text (running list + reminders).

use super::hub::{ExitNotice, MAX_SUBAGENTS_PER_PARENT, RunningJob};

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Compact elapsed label for the running list: "45s", "3m12s", "1h02m".
fn elapsed_label(started_at_ms: i64, now_ms: i64) -> String {
    let secs = now_ms.saturating_sub(started_at_ms).max(0) as u64 / 1000;
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m{:02}s", secs / 60, secs % 60)
    } else {
        format!("{}h{:02}m", secs / 3600, (secs % 3600) / 60)
    }
}

pub fn format_running_list(jobs: &[RunningJob]) -> String {
    let mut out = format!("running: {}/{}\n", jobs.len(), MAX_SUBAGENTS_PER_PARENT);
    let now = now_ms();
    for j in jobs {
        out.push_str(&format!(
            "- {}  {}  {}  {}\n",
            j.id,
            j.agent_name,
            elapsed_label(j.started_at_ms, now),
            j.prompt_preview
        ));
    }
    out
}

pub fn guidance_line() -> &'static str {
    "Use subagent_list to list sessions. subagent_wait to wait. subagent_stop to cancel the current turn. session_search to read a child's transcript."
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

pub fn format_completed_status(notice: &ExitNotice, jobs: &[RunningJob]) -> String {
    let mut out = String::new();
    if notice.stopped {
        out.push_str("status: cancelled\n");
    } else if notice.ok {
        out.push_str("status: completed\n");
    } else {
        out.push_str("status: failed\n");
    }
    out.push_str(&format!("child_session_id: {}\n", notice.child_session_id));
    out.push_str(&format!("agent: {}\n", notice.agent_name));
    if !notice.final_text.is_empty() {
        out.push_str("output:\n");
        out.push_str(&notice.final_text);
        if !notice.final_text.ends_with('\n') {
            out.push('\n');
        }
    }
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
    if !notice.ok && !notice.stopped {
        out.push_str("ok: false\n");
    }
    if !notice.prompt_preview.is_empty() {
        out.push_str(&format!("task: {}\n", notice.prompt_preview));
    }
    if !notice.final_text.is_empty() {
        out.push_str("output:\n");
        out.push_str(&notice.final_text);
        if !notice.final_text.ends_with('\n') {
            out.push('\n');
        }
    }
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
    let outcome = if notice.stopped {
        "cancelled"
    } else if notice.ok {
        "completed"
    } else {
        "failed"
    };
    out.push_str(&format!("outcome: {outcome}\n"));
    if !notice.prompt_preview.is_empty() {
        out.push_str(&format!("task: {}\n", notice.prompt_preview));
    }
    if !notice.ok && !notice.stopped && !notice.final_text.is_empty() {
        let reason: String = notice.final_text.chars().take(200).collect();
        out.push_str(&format!("reason: {reason}\n"));
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

pub fn format_exit_reminder(notices: &[ExitNotice], jobs: &[RunningJob]) -> String {
    let mut inner = String::new();
    for n in notices {
        if n.stopped {
            inner.push_str(&format!(
                "Subagent {} ({}) was stopped.\ntask: {}\n",
                n.child_session_id, n.agent_name, n.prompt_preview
            ));
        } else if n.ok {
            inner.push_str(&format!(
                "Subagent {} ({}) finished.\ntask: {}\n",
                n.child_session_id, n.agent_name, n.prompt_preview
            ));
        } else {
            inner.push_str(&format!(
                "Subagent {} ({}) failed.\ntask: {}\n",
                n.child_session_id, n.agent_name, n.prompt_preview
            ));
        }
        if !n.final_text.is_empty() {
            let preview: String = n.final_text.chars().take(240).collect();
            inner.push_str(&format!("output_preview: {preview}\n"));
        }
    }
    inner.push_str(&format_running_list(jobs));
    inner.push_str(
        "Use session_search to read the child transcript. subagent_wait / subagent_stop for remaining workers.\n",
    );
    format!("<system-reminder>\n{}</system-reminder>", inner.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::subagent::hub::RunningJob;

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
    }

    #[test]
    fn elapsed_label_is_compact() {
        assert_eq!(elapsed_label(0, 0), "0s");
        assert_eq!(elapsed_label(0, 45_000), "45s");
        assert_eq!(elapsed_label(0, 192_000), "3m12s");
        assert_eq!(elapsed_label(0, 3_720_000), "1h02m");
    }
}

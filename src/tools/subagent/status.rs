//! Agent-facing subagent job status text (running list + reminders).

use super::hub::{ExitNotice, RunningJob};

pub fn format_running_list(jobs: &[RunningJob]) -> String {
    if jobs.is_empty() {
        return "running: 0\n".into();
    }
    let mut out = format!("running: {}\n", jobs.len());
    for j in jobs {
        out.push_str(&format!("- {}  {}  {}\n", j.id, j.agent_name, j.prompt_preview));
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
    let mut out = String::from("status: waited\n");
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

pub fn format_stopped_status(child_id: &str, jobs: &[RunningJob]) -> String {
    let mut msg = format!("Stopped subagent '{child_id}'.\n");
    msg.push_str(&format_running_list(jobs));
    msg.push_str(guidance_line());
    msg.push('\n');
    msg
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
        }];
        let got = format_running_status("child-a", &jobs);
        assert!(got.starts_with("status: running\nchild_session_id: child-a\n"));
        assert!(got.contains("- child-a  reviewer  review this\n"));
        assert!(got.contains(guidance_line()));
    }
}

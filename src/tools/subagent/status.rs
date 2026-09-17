//! Agent-facing rendering of Session facts.

use crate::session::manager::SessionManager;
use crate::session::model::TurnResult;

use super::jobs::CompletionRef;

const REPORT_BUDGET: usize = 24_000;

pub fn format_started(child_id: &str, turn_id: &str, responsibility: Option<&str>) -> String {
    let mut out = format!("status: running\nchild_session_id: {child_id}\nturn_id: {turn_id}\n");
    if let Some(responsibility) = responsibility.filter(|value| !value.is_empty()) {
        out.push_str(&format!("responsibility: {responsibility}\n"));
    }
    out.push_str(
        "The child runs in the background; its result will be delivered when this turn settles.\n",
    );
    out
}

pub fn format_stop_requested(child_id: &str, turn_id: &str) -> String {
    format!(
        "status: stop_requested\nchild_session_id: {child_id}\nturn_id: {turn_id}\n\
         The current work may already have consumed resources. Confirm the next assignment before continuing this session.\n"
    )
}

pub fn format_unknown_child(child_id: &str) -> String {
    format!("subagent '{child_id}' is not a child session of this session")
}

pub fn format_turn_result(
    child_id: &str,
    agent: Option<&str>,
    responsibility: Option<&str>,
    result: &TurnResult,
) -> String {
    format_turn_result_with_budget(child_id, agent, responsibility, result, REPORT_BUDGET)
}

pub fn session_labels(
    sessions: &SessionManager,
    child_id: &str,
) -> (Option<String>, Option<String>) {
    let agent = sessions.agent_id(child_id).filter(|value| !value.is_empty());
    let responsibility = sessions
        .reader()
        .meta_blocking(child_id)
        .ok()
        .map(|meta| meta.responsibility)
        .filter(|value| !value.is_empty());
    (agent, responsibility)
}

fn format_turn_result_with_budget(
    child_id: &str,
    agent: Option<&str>,
    responsibility: Option<&str>,
    result: &TurnResult,
    budget: usize,
) -> String {
    let mut out = format!(
        "child_session_id: {child_id}\nturn_id: {}\nreason: {}\n",
        result.turn_id, result.reason
    );
    if let Some(agent) = agent.filter(|value| !value.is_empty()) {
        out.push_str(&format!("agent: {agent}\n"));
    }
    if let Some(responsibility) = responsibility.filter(|value| !value.is_empty()) {
        out.push_str(&format!("responsibility: {responsibility}\n"));
    }
    if result.output.is_empty() {
        return out;
    }

    let fixed_tail = truncation_location(result);
    let remaining = budget.saturating_sub(out.len() + fixed_tail.len() + 32);
    if result.output.len() <= remaining {
        out.push_str("output:\n");
        out.push_str(&result.output);
        if !result.output.ends_with('\n') {
            out.push('\n');
        }
        return out;
    }

    let mut kept = String::new();
    for line in result.output.lines() {
        let addition = line.len() + 1;
        if kept.len() + addition > remaining {
            break;
        }
        kept.push_str(line);
        kept.push('\n');
    }
    out.push_str("output:\n");
    out.push_str(&kept);
    out.push_str("output_truncated: true\n");
    out.push_str(&fixed_tail);
    out
}

fn truncation_location(result: &TurnResult) -> String {
    match (result.start_line, result.end_line) {
        (Some(start), Some(end)) => format!(
            "transcript: {}\nstart_line: {start}\nend_line: {end}\n\
             read: {{\"file_path\":\"{}\",\"start_line\":{start},\"end_line\":{end}}}\n",
            result.transcript_path, result.transcript_path
        ),
        _ => format!("transcript: {}\n", result.transcript_path),
    }
}

pub fn format_batch_results(sessions: &SessionManager, completions: &[CompletionRef]) -> String {
    if completions.is_empty() {
        return "status: nothing to wait for\n".into();
    }
    let mut out = format!("status: settled\nsettled: {}\n", completions.len());
    for completion in completions {
        out.push_str("---\n");
        match sessions
            .data()
            .turn_result_blocking(&completion.child_session_id, &completion.turn_id)
        {
            Ok(result) => {
                let (agent, responsibility) =
                    session_labels(sessions, &completion.child_session_id);
                out.push_str(&format_turn_result(
                    &completion.child_session_id,
                    agent.as_deref(),
                    responsibility.as_deref(),
                    &result,
                ))
            }
            Err(error) => out.push_str(&format!(
                "child_session_id: {}\nturn_id: {}\nreason: unknown\nresult_error: {error}\n",
                completion.child_session_id, completion.turn_id
            )),
        }
    }
    out
}

pub fn format_wait_outcome(
    sessions: &SessionManager,
    completions: &[CompletionRef],
    skipped: &[(String, String)],
) -> String {
    if completions.is_empty() {
        let mut out = String::from("status: nothing to wait for\n");
        append_skipped(&mut out, skipped);
        return out;
    }
    let mut out = format_batch_results(sessions, completions);
    append_skipped(&mut out, skipped);
    out
}

fn append_skipped(out: &mut String, skipped: &[(String, String)]) {
    if skipped.is_empty() {
        return;
    }
    out.push_str(&format!("skipped: {}\n", skipped.len()));
    for (id, reason) in skipped {
        out.push_str("---\n");
        out.push_str(&format!("skipped_id: {id}\nskipped_reason: {reason}\n"));
    }
}

pub fn format_completion_reminder(
    sessions: &SessionManager,
    completions: &[CompletionRef],
) -> String {
    let mut inner = String::from(
        "source: subagent\nThe following background child session turns settled.\n",
    );
    inner.push_str(&format_batch_results(sessions, completions));
    format!("<system-reminder>\n{}</system-reminder>", inner.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncation_is_explicit_and_actionable() {
        let result = TurnResult {
            turn_id: "t".into(),
            reason: "completed".into(),
            output: "line\n".repeat(100),
            transcript_path: ".litecode/sessions/c.md".into(),
            start_line: Some(12),
            end_line: Some(111),
        };
        let text = format_turn_result_with_budget(
            "c",
            Some("reviewer"),
            Some("review the diff"),
            &result,
            180,
        );
        assert!(text.contains("responsibility: review the diff"));
        assert!(text.contains("output_truncated: true"));
        assert!(text.contains("\"start_line\":12"));
        assert!(text.contains("\"end_line\":111"));
    }
}

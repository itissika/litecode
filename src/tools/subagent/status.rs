//! Agent-facing rendering of Session facts.

use crate::session::SessionService;
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

pub fn format_queued(child_id: &str, position: usize) -> String {
    format!(
        "status: queued\nchild_session_id: {child_id}\nposition: {position}\n\
         The child is running; this message runs as its own new turn after the current turn ends.\n"
    )
}

pub fn format_steered(child_id: &str, pending_id: &str) -> String {
    format!(
        "status: steered\nchild_session_id: {child_id}\npending_id: {pending_id}\n\
         The child is running; this message is injected into its current turn at the next request.\n"
    )
}

pub fn format_stopped_and_queued(child_id: &str) -> String {
    format!(
        "status: stopped_and_queued\nchild_session_id: {child_id}\n\
         The current turn was stopped; this message runs as the next new turn.\n"
    )
}

pub fn format_stopped(child_id: &str) -> String {
    format!(
        "status: stopped\nchild_session_id: {child_id}\n\
         Only the current turn was stopped; queued messages are still delivered. \
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
    service: &SessionService,
    child_id: &str,
) -> (Option<String>, Option<String>) {
    let agent = service.agent_id(child_id).filter(|value| !value.is_empty());
    let responsibility = service
        .meta(child_id)
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
             read: {{\"path\":\"{}\",\"start_line\":{start},\"end_line\":{end}}}\n",
            result.transcript_path, result.transcript_path
        ),
        _ => format!("transcript: {}\n", result.transcript_path),
    }
}

pub fn format_batch_results(service: &SessionService, completions: &[CompletionRef]) -> String {
    if completions.is_empty() {
        return "status: nothing to wait for\n".into();
    }
    let mut out = format!("status: settled\nsettled: {}\n", completions.len());
    for completion in completions {
        out.push_str("---\n");
        match service.turn_result(&completion.child_session_id, &completion.turn_id) {
            Ok(result) => {
                let (agent, responsibility) = session_labels(service, &completion.child_session_id);
                out.push_str(&format_turn_result(
                    &completion.child_session_id,
                    agent.as_deref(),
                    responsibility.as_deref(),
                    &result,
                ));
                if result.reason == "cancelled" || completion.cancelled {
                    out.push_str(&format!(
                        "cancelled_by: {}\n",
                        completion
                            .cancelled_by
                            .map(|by| by.as_str())
                            .unwrap_or("null")
                    ));
                }
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
    service: &SessionService,
    completions: &[CompletionRef],
    skipped: &[(String, String)],
) -> String {
    if completions.is_empty() {
        let mut out = String::from("status: nothing to wait for\n");
        append_skipped(&mut out, skipped);
        return out;
    }
    let mut out = format_batch_results(service, completions);
    append_skipped(&mut out, skipped);
    out
}

/// Merge `reason` and `cancelled_by` into one flat string so model-facing data
/// does not carry a mostly-empty sparse `cancelled_by` column.
pub fn merged_reason(
    reason: &str,
    cancelled: bool,
    cancelled_by: Option<crate::types::StopOrigin>,
) -> String {
    if reason == "cancelled" || cancelled {
        match cancelled_by {
            Some(crate::types::StopOrigin::User) => "cancelled_by_user".to_string(),
            Some(crate::types::StopOrigin::Parent) => "cancelled_by_parent".to_string(),
            None => "cancelled".to_string(),
        }
    } else {
        reason.to_string()
    }
}

pub fn format_wait_outcome_data(
    service: &SessionService,
    completions: &[CompletionRef],
    skipped: &[(String, String)],
) -> (serde_json::Value, Option<serde_json::Value>) {
    let mut truncated_children: Vec<serde_json::Value> = Vec::new();
    let settled: Vec<serde_json::Value> = completions
        .iter()
        .map(|completion| match service.turn_result(&completion.child_session_id, &completion.turn_id) {
            Ok(result) => {
                let (_, responsibility) = session_labels(service, &completion.child_session_id);
                let mut obj = serde_json::Map::new();
                obj.insert(
                    "id".into(),
                    serde_json::Value::String(completion.child_session_id.clone()),
                );
                obj.insert(
                    "reason".into(),
                    serde_json::Value::String(merged_reason(
                        &result.reason,
                        completion.cancelled,
                        completion.cancelled_by,
                    )),
                );
                if let Some(resp) = responsibility.filter(|v| !v.is_empty()) {
                    obj.insert("responsibility".into(), serde_json::Value::String(resp));
                }
                let (output, truncated) = bounded_output_excerpt(&result.output);
                obj.insert("output".into(), serde_json::Value::String(output));
                if truncated {
                    let mut meta_row = serde_json::Map::new();
                    meta_row.insert(
                        "id".into(),
                        serde_json::Value::String(completion.child_session_id.clone()),
                    );
                    if !result.transcript_path.is_empty() {
                        meta_row.insert(
                            "transcript".into(),
                            serde_json::Value::String(result.transcript_path.clone()),
                        );
                    }
                    if let Some(start) = result.start_line {
                        meta_row.insert("start_line".into(), serde_json::json!(start));
                    }
                    if let Some(end) = result.end_line {
                        meta_row.insert("end_line".into(), serde_json::json!(end));
                    }
                    truncated_children.push(serde_json::Value::Object(meta_row));
                }
                serde_json::Value::Object(obj)
            }
            Err(error) => serde_json::json!({
                "id": completion.child_session_id,
                "reason": "unknown",
                "output": format!("result unavailable: {error}"),
            }),
        })
        .collect();
    let mut root = serde_json::Map::new();
    root.insert("settled".into(), serde_json::Value::Array(settled));
    if !skipped.is_empty() {
        let skipped_json: Vec<serde_json::Value> = skipped
            .iter()
            .map(|(id, reason)| {
                serde_json::json!({
                    "id": id,
                    "reason": reason,
                })
            })
            .collect();
        root.insert("skipped".into(), serde_json::Value::Array(skipped_json));
    }
    let metadata = (!truncated_children.is_empty())
        .then(|| serde_json::json!({ "truncated_children": truncated_children }));
    (serde_json::Value::Object(root), metadata)
}

fn bounded_output_excerpt(output: &str) -> (String, bool) {
    if output.len() <= REPORT_BUDGET {
        return (output.to_string(), false);
    }
    let mut kept = String::new();
    for line in output.lines() {
        if kept.len() + line.len() + 1 > REPORT_BUDGET {
            break;
        }
        kept.push_str(line);
        kept.push('\n');
    }
    (kept, true)
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

#[cfg(test)]
mod tests {}

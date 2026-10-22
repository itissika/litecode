//! Model-facing text for one `run_code` execution.
//!
//! Default agent view is only three blocks: head, optional logs, optional result.
//! The view stays inside [`RUN_CODE_VIEW_MAX_BYTES`]. Cuts land on UTF-8
//! boundaries and say what was omitted. This module does not emit a JSON copy
//! of the run.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::ptc::{FailureKind, ResourceKind, SourceLocation};
use crate::session::transcript_file::RUN_CODE_SESSION_NOTE;
use crate::tool::outcome::Diagnostic;
use crate::types::utf8::{prefix_utf8, suffix_utf8};

/// Hard cap for the model-facing `run_code` view, including omission notes.
pub const RUN_CODE_VIEW_MAX_BYTES: usize = 65_536;

const RESERVE_BYTES: usize = 512;
const HEADER_CAP: usize = 1024;
const LOGS_FLOOR: usize = 12 * 1024;
const HEAD_MESSAGE_CAP: usize = 240;

/// One nested call as counted for the head. Kept for host/tests; not rendered
/// as a ledger section in the default model view.
pub(super) struct LedgerSnap {
    pub seq: u64,
    pub tool: String,
    pub summary: String,
    pub state: String,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub session_read: bool,
}

/// An uncaught `ToolError`, used for the header failure tag.
pub(super) struct UncaughtToolError {
    pub seq: u64,
    pub tool: String,
    pub code: String,
    pub message: String,
    pub summary: String,
    pub details: Option<Value>,
}

/// Facts the renderer needs. Built from the in-memory execution report.
pub(super) struct RunReport {
    pub logs: Vec<(String, String)>,
    pub value: Option<Value>,
    pub failure_text: Option<String>,
    pub failure_kind: Option<FailureKind>,
    pub resource: Option<ResourceKind>,
    pub timeout_ms: u64,
    pub duration_ms: u64,
    pub output_limited: bool,
    pub calls: Vec<LedgerSnap>,
    pub calls_omitted: usize,
    /// Retained for tests/human channels; never rendered into the default view.
    pub diagnostics: Vec<Diagnostic>,
    pub diagnostics_omitted: usize,
    pub tool_counts: BTreeMap<String, usize>,
    /// Settled ok/fail per tool (human/FE aggregate; not rendered into content).
    pub by_tool: BTreeMap<String, (usize, usize)>,
    pub error_counts: BTreeMap<String, usize>,
    pub call_total: Option<usize>,
    pub unsettled_total: Option<usize>,
    pub session_read_hidden: bool,
    pub failure_location: Option<SourceLocation>,
    pub failure_at_end: bool,
    pub failure_stack: Option<String>,
    pub source: String,
    pub tool_error: Option<UncaughtToolError>,
    pub uncaught_type_error: bool,
    /// `dropped undefined at …` head line (WP-206 #7).
    pub dropped_undefined: Option<String>,
}

pub(super) fn render_model_view(report: &RunReport) -> String {
    let fixed = vec![render_header_block(report)];
    let fixed_len = joined_len(&fixed);
    let result_body = report.value.as_ref().map(render_result_body);
    let has_logs = !report.logs.is_empty();
    let mut logs_cap = logs_budget(fixed_len, result_body.as_deref(), has_logs);
    let mut text;
    let mut guard = 0;
    loop {
        let logs = render_logs(&report.logs, logs_cap, report.output_limited);
        // `remaining_after` already reserved the separator before this section.
        // The extra 2 bytes are the separator between logs and result, which
        // `logs_budget` also reserved when a result is present.
        let used_logs = logs.as_ref().map(|logs| logs.len() + 2).unwrap_or(0);
        let mut result_cap = remaining_after(fixed_len).saturating_sub(used_logs);
        let mut result = render_result(result_body.as_deref(), result_cap);
        text = assemble(&fixed, &logs, &result);
        while text.len() > RUN_CODE_VIEW_MAX_BYTES && result_cap > 0 {
            let over = text.len() - RUN_CODE_VIEW_MAX_BYTES;
            result_cap = result_cap.saturating_sub(over.max(64));
            result = render_result(result_body.as_deref(), result_cap);
            text = assemble(&fixed, &logs, &result);
        }
        if text.len() <= RUN_CODE_VIEW_MAX_BYTES || !has_logs || logs_cap == 0 || guard == 8 {
            break;
        }
        guard += 1;
        let over = text.len() - RUN_CODE_VIEW_MAX_BYTES;
        logs_cap = logs_cap.saturating_sub(over.max(64));
    }
    if text.len() > RUN_CODE_VIEW_MAX_BYTES {
        text = explained_cut(&text, RUN_CODE_VIEW_MAX_BYTES);
    }
    text
}

/// Human / FE aggregate for `ToolCallResult.data`. Never mirrored into model content.
///
/// - Program started: full `{succeeded,failed,duration_ms,by_tool}` (+ optional ok /
///   failure_kind / unsettled). Counts match the head (settled only).
/// - Won't-start (syntax / setup): ultra-minimal `{ok:false, failure_kind}` only.
pub(super) fn human_data(report: &RunReport) -> Option<Value> {
    if did_not_start(report) {
        let mut map = serde_json::Map::new();
        map.insert("ok".into(), Value::Bool(false));
        if let Some(kind) = report.failure_kind {
            map.insert(
                "failure_kind".into(),
                Value::String(failure_kind_slug(kind).into()),
            );
        }
        return Some(Value::Object(map));
    }

    let (succeeded, failed) = success_failure_counts(report);
    let unsettled = unsettled_total(report);
    let by_tool = by_tool_json(report);
    let mut map = serde_json::Map::new();
    map.insert("succeeded".into(), Value::from(succeeded as u64));
    map.insert("failed".into(), Value::from(failed as u64));
    map.insert("duration_ms".into(), Value::from(report.duration_ms));
    map.insert("by_tool".into(), by_tool);
    let ok = report.failure_kind.is_none();
    map.insert("ok".into(), Value::Bool(ok));
    if let Some(kind) = report.failure_kind {
        map.insert(
            "failure_kind".into(),
            Value::String(failure_kind_slug(kind).into()),
        );
    }
    if unsettled > 0 {
        map.insert("unsettled".into(), Value::from(unsettled as u64));
    }
    Some(Value::Object(map))
}

fn by_tool_json(report: &RunReport) -> Value {
    let mut map = serde_json::Map::new();
    for (tool, (ok, fail)) in &report.by_tool {
        map.insert(
            tool.clone(),
            serde_json::json!({ "ok": ok, "fail": fail }),
        );
    }
    Value::Object(map)
}

fn failure_kind_slug(kind: FailureKind) -> &'static str {
    match kind {
        FailureKind::Syntax => "syntax",
        FailureKind::Exception => "exception",
        FailureKind::Timeout => "timeout",
        FailureKind::Cancelled => "cancelled",
        FailureKind::ResourceLimit => "resource_limit",
        FailureKind::OutputLimit => "output_limit",
        FailureKind::IllegalOutput => "illegal_output",
        FailureKind::Internal => "internal",
    }
}

fn assemble(fixed: &[String], logs: &Option<String>, result: &Option<String>) -> String {
    let mut sections = fixed.to_vec();
    if let Some(logs) = logs {
        sections.push(logs.clone());
    }
    if let Some(result) = result {
        sections.push(result.clone());
    }
    sections.join("\n\n")
}

fn joined_len(sections: &[String]) -> usize {
    if sections.is_empty() {
        return 0;
    }
    sections.iter().map(String::len).sum::<usize>() + 2 * (sections.len() - 1)
}

fn remaining_after(fixed_len: usize) -> usize {
    let separator = if fixed_len == 0 { 0 } else { 2 };
    RUN_CODE_VIEW_MAX_BYTES
        .saturating_sub(fixed_len)
        .saturating_sub(separator)
        .saturating_sub(RESERVE_BYTES)
}

fn logs_budget(fixed_len: usize, result_body: Option<&str>, has_logs: bool) -> usize {
    if !has_logs {
        return 0;
    }
    let room = remaining_after(fixed_len);
    // Reserve the result section plus the separator in front of it. Otherwise a
    // log section that fills this budget leaves the result 2 bytes short and a
    // value that would have fit verbatim gets cut.
    let result_reserve = result_body
        .map(|body| result_section_len(body).saturating_add(2))
        .unwrap_or(0);
    room.saturating_sub(result_reserve)
        .max(LOGS_FLOOR)
        .min(room)
}

fn result_section_len(body: &str) -> usize {
    "result:\n".len() + body.len()
}

fn render_header_block(report: &RunReport) -> String {
    let mut parts = vec![header_line(report)];
    if let Some(line) = &report.dropped_undefined {
        parts.push(line.clone());
    }
    if !did_not_start(report) {
        let unsettled = unsettled_total(report);
        if unsettled > 0 {
            parts.push(format!(
                "{unsettled} calls were still running when the program stopped; their side effects may still land. Re-read the files before retrying any write."
            ));
        }
    }
    explained_cut(&parts.join("\n\n"), HEADER_CAP)
}

fn call_total(report: &RunReport) -> usize {
    report
        .call_total
        .unwrap_or(report.calls.len() + report.calls_omitted)
}

fn unsettled_total(report: &RunReport) -> usize {
    report.unsettled_total.unwrap_or_else(|| {
        report
            .calls
            .iter()
            .filter(|call| call.state == "unsettled")
            .count()
    })
}

fn success_failure_counts(report: &RunReport) -> (usize, usize) {
    // Settled calls only. Caught rejects (`error:*`) count as failures.
    // Unsettled is excluded from both numbers and noted separately on the head.
    // Host `cancelled` is settled but neither success nor reject — excluded too.
    let failed = if !report.error_counts.is_empty() {
        report.error_counts.values().copied().sum()
    } else {
        report
            .calls
            .iter()
            .filter(|call| call.state.starts_with("error"))
            .count()
    };
    let complete_snaps = report.calls_omitted == 0
        && report
            .call_total
            .is_none_or(|total| total == report.calls.len());
    let success = if complete_snaps {
        report
            .calls
            .iter()
            .filter(|call| call.state == "ok" || call.state.starts_with("ok("))
            .count()
    } else {
        // Omitted rows: approximate successes as settled minus known rejects.
        // Cancelled-but-settled among omitted is rare; prefer not inventing them.
        let total = call_total(report);
        let unsettled = unsettled_total(report);
        total.saturating_sub(unsettled).saturating_sub(failed)
    };
    (success, failed)
}

/// Syntax / setup failures never drove the program. Head is line + cause only.
fn did_not_start(report: &RunReport) -> bool {
    match report.failure_kind {
        Some(FailureKind::Syntax) => true,
        Some(FailureKind::Internal)
            if call_total(report) == 0 && report.logs.is_empty() && report.value.is_none() =>
        {
            true
        }
        _ => false,
    }
}

fn header_line(report: &RunReport) -> String {
    if did_not_start(report) {
        return wont_start_line(report);
    }
    let (success, failed) = success_failure_counts(report);
    let duration = format!("{}ms", report.duration_ms);
    let mut line = if let Some(error) = &report.tool_error {
        format!(
            "run_code failed: uncaught ToolError {} from call #{} {} {} · {success} succeeded, {failed} failed · {duration}",
            error.code, error.seq, error.tool, error.summary
        )
    } else if let Some(kind) = report.failure_kind {
        let label = if report.uncaught_type_error {
            "uncaught TypeError".to_string()
        } else {
            failure_label(kind, report)
        };
        let mut line = format!(
            "run_code failed: {label} · {success} succeeded, {failed} failed · {duration}"
        );
        // Exception / illegal-output: one short cause on the head. Timeout /
        // cancel / limits already have a precise label — do not rehash.
        if matches!(
            kind,
            FailureKind::Exception | FailureKind::IllegalOutput | FailureKind::Internal
        ) {
            if let Some(detail) = short_failure_detail(report) {
                line.push_str(" · ");
                line.push_str(&detail);
            }
        }
        line
    } else {
        format!("run_code ok · {success} succeeded, {failed} failed · {duration}")
    };
    if session_read_mark(report) {
        line.push(' ');
        line.push_str(RUN_CODE_SESSION_NOTE);
    }
    line
}

fn wont_start_line(report: &RunReport) -> String {
    let label = match report.failure_kind {
        Some(kind) => failure_label(kind, report),
        None => "failed".to_string(),
    };
    let mut line = format!("run_code failed: {label}");
    if report.failure_at_end {
        line.push_str(" · at end of input");
    } else if let Some(location) = report.failure_location {
        line.push_str(&format!(
            " · line {}, column {}",
            location.line, location.column
        ));
        if let Some(snippet) = source_line_snippet(&report.source, location.line) {
            line.push_str(" · ");
            line.push_str(&snippet);
        }
    }
    if let Some(detail) = short_failure_detail(report) {
        line.push_str(": ");
        line.push_str(&detail);
    }
    line
}

fn source_line_snippet(source: &str, line: u32) -> Option<String> {
    let raw = source.lines().nth(line.saturating_sub(1) as usize)?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(prefix_utf8(trimmed, 120).to_string())
}

fn short_failure_detail(report: &RunReport) -> Option<String> {
    let text = report.failure_text.as_deref()?.trim();
    if text.is_empty() {
        return None;
    }
    // Prefer the first line; drop redundant "run_code timed out…" when the
    // label already names the timeout.
    let first = text.lines().next().unwrap_or(text).trim();
    if first.is_empty() {
        return None;
    }
    if report.failure_kind == Some(FailureKind::Timeout)
        && first.starts_with("run_code timed out")
    {
        return None;
    }
    Some(prefix_utf8(first, HEAD_MESSAGE_CAP).to_string())
}

fn session_read_mark(report: &RunReport) -> bool {
    report.session_read_hidden || report.calls.iter().any(|call| call.session_read)
}

fn failure_label(kind: FailureKind, report: &RunReport) -> String {
    match kind {
        FailureKind::Timeout => format!("timeout after {}ms", report.timeout_ms),
        FailureKind::Cancelled if report.value.is_some() => {
            "cancelled after the program returned".to_string()
        }
        FailureKind::Cancelled => "cancelled".to_string(),
        FailureKind::OutputLimit => "output limit".to_string(),
        FailureKind::ResourceLimit => match report.resource {
            Some(ResourceKind::Stack) => "stack overflow".to_string(),
            _ => "out of memory".to_string(),
        },
        FailureKind::Syntax => "syntax error".to_string(),
        FailureKind::IllegalOutput => "illegal output".to_string(),
        FailureKind::Exception => "exception".to_string(),
        FailureKind::Internal => "internal error".to_string(),
    }
}

const OUTPUT_LIMIT_MARK: &str = " [output limit reached]";

fn render_logs(logs: &[(String, String)], cap: usize, output_limited: bool) -> Option<String> {
    if logs.is_empty() || cap == 0 {
        return None;
    }
    let lines: Vec<String> = logs
        .iter()
        .map(|(level, message)| format!("[{level}] {message}"))
        .collect();
    let mut start = lines.len();
    let mut best: Option<String> = None;
    while start > 0 {
        start -= 1;
        let body = logs_body(&lines, start, output_limited);
        if body.len() <= cap {
            best = Some(body);
            continue;
        }
        if best.is_none() {
            let fitted = if output_limited {
                fit_limited_last_line(&lines, cap)
            } else {
                explained_cut(&body, cap)
            };
            return nonempty(fitted);
        }
        break;
    }
    best
}

fn logs_body(lines: &[String], start: usize, output_limited: bool) -> String {
    let omitted = &lines[..start];
    let mut body = String::from("logs:");
    if !omitted.is_empty() {
        let bytes: usize = omitted.iter().map(String::len).sum();
        body.push('\n');
        body.push_str(&format!(
            "… {} earlier log lines ({bytes} bytes) omitted",
            omitted.len()
        ));
    }
    for (offset, line) in lines[start..].iter().enumerate() {
        body.push('\n');
        body.push_str(line);
        if output_limited && start + offset + 1 == lines.len() {
            body.push_str(OUTPUT_LIMIT_MARK);
        }
    }
    body
}

/// The limit mark is appended after the UTF-8 cut, so a prefix trim cannot drop it.
fn fit_limited_last_line(lines: &[String], cap: usize) -> String {
    let Some(last) = lines.last() else {
        return String::new();
    };
    let omitted_count = lines.len() - 1;
    let mut prefix = String::from("logs:");
    if omitted_count > 0 {
        let bytes: usize = lines[..omitted_count].iter().map(String::len).sum();
        prefix.push('\n');
        prefix.push_str(&format!(
            "… {omitted_count} earlier log lines ({bytes} bytes) omitted"
        ));
    }
    prefix.push('\n');
    if prefix.len().saturating_add(OUTPUT_LIMIT_MARK.len()) > cap {
        return String::new();
    }
    let room = cap - prefix.len() - OUTPUT_LIMIT_MARK.len();
    let cut = prefix_utf8(last, room);
    let mut body = prefix;
    body.push_str(cut);
    body.push_str(OUTPUT_LIMIT_MARK);
    body
}

fn render_result(body: Option<&str>, cap: usize) -> Option<String> {
    let body = body?;
    if cap == 0 {
        return None;
    }
    let full = format!("result:\n{body}");
    if full.len() <= cap {
        return Some(full);
    }
    nonempty(fit_result(body, cap))
}

fn render_result_body(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => serde_json::to_string_pretty(other).unwrap_or_else(|_| other.to_string()),
    }
}

fn fit_result(body: &str, cap: usize) -> String {
    let header = "result:\n";
    if cap <= header.len() {
        return explained_cut(header, cap);
    }
    let room = cap - header.len();
    let marker_room = omission_marker(body.len(), body.len()).len();
    let available = room.saturating_sub(marker_room);
    if available == 0 {
        return explained_cut(&format!("{header}{body}"), cap);
    }
    let head_budget = available * 2 / 3;
    let tail_budget = available - head_budget;
    let head = prefix_utf8(body, head_budget);
    let tail = suffix_utf8(body, tail_budget);
    let (head, tail) = disjoint_ends(body, head, tail);
    let omitted = body.len().saturating_sub(head.len() + tail.len());
    let marker = omission_marker(omitted, body.len());
    let rendered = format!("{header}{head}{marker}{tail}");
    if rendered.len() <= cap {
        rendered
    } else {
        explained_cut(&rendered, cap)
    }
}

fn disjoint_ends<'a>(body: &'a str, head: &'a str, tail: &'a str) -> (&'a str, &'a str) {
    if head.len() + tail.len() <= body.len() && body.starts_with(head) && body.ends_with(tail) {
        (head, tail)
    } else {
        (body, "")
    }
}

fn omission_marker(omitted: usize, total: usize) -> String {
    format!("… [{omitted} bytes omitted of {total}] …")
}

/// Cut `text` to `cap` and say what was omitted. Never returns an unmarked prefix.
/// If the omission note itself does not fit, the section is omitted.
fn explained_cut(text: &str, cap: usize) -> String {
    if text.len() <= cap {
        return text.to_string();
    }
    let total = text.len();
    let mut keep = cap.min(total);
    loop {
        let prefix = prefix_utf8(text, keep);
        let omitted = total - prefix.len();
        let note = format!("… [{omitted} bytes omitted of {total}]");
        if prefix.len() + note.len() <= cap {
            return format!("{prefix}{note}");
        }
        if prefix.is_empty() {
            return String::new();
        }
        let overflow = (prefix.len() + note.len()).saturating_sub(cap).max(1);
        let next = prefix.len().saturating_sub(overflow);
        if next == 0 {
            keep = 0;
            continue;
        }
        keep = next;
    }
}

fn nonempty(text: String) -> Option<String> {
    if text.is_empty() { None } else { Some(text) }
}

#[cfg(test)]
mod tests {
    use super::*;


    fn report() -> RunReport {
        RunReport {
            logs: Vec::new(),
            value: None,
            failure_text: None,
            failure_kind: None,
            resource: None,
            timeout_ms: 120_000,
            duration_ms: 12,
            output_limited: false,
            calls: Vec::new(),
            calls_omitted: 0,
            diagnostics: Vec::new(),
            diagnostics_omitted: 0,
            tool_counts: BTreeMap::new(),
            by_tool: BTreeMap::new(),
            error_counts: BTreeMap::new(),
            call_total: None,
            unsettled_total: None,
            session_read_hidden: false,
            failure_location: None,
            failure_at_end: false,
            failure_stack: None,
            source: String::new(),
            tool_error: None,
            uncaught_type_error: false,
            dropped_undefined: None,
        }
    }

    #[test]
    fn dropped_undefined_line_sits_in_the_head() {
        let mut report = report();
        report.value = Some(Value::Null);
        report.dropped_undefined = Some("dropped undefined at a.b, c[2]".into());
        let text = render_model_view(&report);
        let head = text.split("result:").next().unwrap_or("");
        assert!(head.contains("dropped undefined at a.b, c[2]"), "{text}");
    }

    #[test]
    fn cjk_result_stays_valid_utf8_inside_the_budget() {
        let mut report = report();
        report.value = Some(Value::String("中".repeat(300_000)));
        let text = render_model_view(&report);
        assert!(std::str::from_utf8(text.as_bytes()).is_ok());
        assert!(text.len() <= RUN_CODE_VIEW_MAX_BYTES, "{}", text.len());
        assert!(text.contains('中'), "{text}");
    }

    #[test]
    fn a_log_that_fills_its_budget_keeps_a_short_result_verbatim() {
        let mut report = report();
        report.logs = vec![("info".into(), "A".repeat(80_000))];
        report.value = Some(Value::String("hi".into()));
        let text = render_model_view(&report);
        assert!(text.ends_with("result:\nhi"), "{text}");
        assert!(text.contains("bytes omitted"), "{text}");
        assert!(text.len() <= RUN_CODE_VIEW_MAX_BYTES, "{}", text.len());
    }

    #[test]
    fn output_limit_mark_survives_an_overlong_last_line() {
        let mut report = report();
        report.output_limited = true;
        report.failure_kind = Some(FailureKind::OutputLimit);
        report.failure_text = Some("output exceeded 1048576 bytes".into());
        report.logs = vec![("info".into(), "中".repeat(40_000))];
        let text = render_model_view(&report);
        assert!(text.contains("[output limit reached]"), "{text}");
        assert!(text.len() <= RUN_CODE_VIEW_MAX_BYTES, "{}", text.len());
        let marked = text
            .lines()
            .find(|line| line.contains("[output limit reached]"))
            .expect("marked log line");
        assert!(marked.ends_with("[output limit reached]"), "{marked}");
        assert!(std::str::from_utf8(text.as_bytes()).is_ok());
    }

}

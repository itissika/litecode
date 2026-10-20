//! Model-facing text for one `run_code` execution.
//!
//! The view stays inside [`RUN_CODE_VIEW_MAX_BYTES`]. Cuts land on UTF-8
//! boundaries and say what was omitted. This module does not emit a JSON copy
//! of the run.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::ptc::{FailureKind, ResourceKind};
use crate::tool::outcome::Diagnostic;
use crate::types::utf8::{prefix_utf8, suffix_utf8};

/// Hard cap for the model-facing `run_code` view, including omission notes.
pub const RUN_CODE_VIEW_MAX_BYTES: usize = 65_536;

const RESERVE_BYTES: usize = 512;
const HEADER_CAP: usize = 1024;
const FAILURE_CAP: usize = 4 * 1024;
const ERRORS_CAP: usize = 3 * 1024;
const DIAGNOSTICS_CAP: usize = 4 * 1024;
const LEDGER_CAP: usize = 8 * 1024;
const LOGS_FLOOR: usize = 12 * 1024;
const ERROR_DETAIL_LIMIT: usize = 8;
const DIAGNOSTIC_RENDER_CAP: usize = 16;
const LSP_TEXT_CAP: usize = 1024;

/// One nested call as the model should see it. `state` contains no spaces.
pub(super) struct LedgerSnap {
    pub seq: u64,
    pub tool: String,
    pub summary: String,
    pub state: String,
    pub error_code: Option<String>,
}

/// Facts the renderer needs. Built from the in-memory execution report.
pub(super) struct RunReport {
    pub logs: Vec<(String, String)>,
    pub value: Option<Value>,
    pub failure_text: Option<String>,
    pub failure_kind: Option<FailureKind>,
    pub resource: Option<ResourceKind>,
    pub timeout_ms: u64,
    pub output_limited: bool,
    pub calls: Vec<LedgerSnap>,
    pub calls_omitted: usize,
    pub diagnostics: Vec<Diagnostic>,
    pub diagnostics_omitted: usize,
}

pub(super) fn render_model_view(report: &RunReport) -> String {
    let fixed = render_fixed(report);
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

fn render_fixed(report: &RunReport) -> Vec<String> {
    let mut sections = Vec::new();
    sections.push(render_header_block(report));
    if let Some(failure) = render_failure(report) {
        sections.push(failure);
    }
    if let Some(errors) = render_error_details(report) {
        sections.push(errors);
    }
    if let Some(diagnostics) = render_diagnostics(report) {
        sections.push(diagnostics);
    }
    if let Some(ledger) = render_ledger(report) {
        sections.push(ledger);
    }
    sections
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
    let unsettled = report
        .calls
        .iter()
        .filter(|call| call.state == "unsettled")
        .count();
    // Dropped calls are not in the snapshot, so a failure cannot state an exact
    // unsettled count. Phase 2 keeps those totals.
    if unsettled > 0 && report.calls_omitted == 0 {
        parts.push(format!(
            "{unsettled} calls were still running when the program stopped; their side effects may still land. Re-read the files before retrying any write."
        ));
    }
    explained_cut(&parts.join("\n\n"), HEADER_CAP)
}

fn header_line(report: &RunReport) -> String {
    let total = report.calls.len() + report.calls_omitted;
    let Some(kind) = report.failure_kind else {
        let mut line = format!("run_code ok · {total} calls");
        if report.calls_omitted == 0 && !report.calls.is_empty() {
            line.push_str(" (");
            line.push_str(&tool_counts(&report.calls));
            line.push(')');
        }
        if report.calls_omitted == 0
            && let Some(rejected) = rejected_clause(&report.calls)
        {
            line.push_str(" · ");
            line.push_str(&rejected);
        }
        return line;
    };
    let label = failure_label(kind, report.resource, report.timeout_ms);
    if report.calls_omitted > 0 {
        return format!("run_code failed: {label} · {total} calls");
    }
    let finished = report
        .calls
        .iter()
        .filter(|call| call.state != "unsettled")
        .count();
    let unsettled = report
        .calls
        .iter()
        .filter(|call| call.state == "unsettled")
        .count();
    format!("run_code failed: {label} · {finished}/{total} calls finished, {unsettled} unsettled")
}

fn failure_label(kind: FailureKind, resource: Option<ResourceKind>, timeout_ms: u64) -> String {
    match kind {
        FailureKind::Timeout => format!("timeout after {timeout_ms}ms"),
        FailureKind::Cancelled => "cancelled".to_string(),
        FailureKind::OutputLimit => "output limit".to_string(),
        FailureKind::ResourceLimit => match resource {
            Some(ResourceKind::Stack) => "stack overflow".to_string(),
            _ => "out of memory".to_string(),
        },
        FailureKind::Syntax => "syntax error".to_string(),
        FailureKind::IllegalOutput => "illegal output".to_string(),
        FailureKind::Exception => "exception".to_string(),
        FailureKind::Internal => "internal error".to_string(),
    }
}

fn rejected_clause(calls: &[LedgerSnap]) -> Option<String> {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for call in calls {
        if call.state == "error" {
            let code = call.error_code.as_deref().unwrap_or("error");
            *counts.entry(code).or_default() += 1;
        }
    }
    if counts.is_empty() {
        return None;
    }
    let total: usize = counts.values().copied().sum();
    let breakdown = counts
        .into_iter()
        .map(|(code, count)| format!("{code}×{count}"))
        .collect::<Vec<_>>()
        .join(", ");
    Some(format!(
        "{total} rejected, handled by the program ({breakdown})"
    ))
}

fn tool_counts(calls: &[LedgerSnap]) -> String {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for call in calls {
        *counts.entry(call.tool.as_str()).or_default() += 1;
    }
    counts
        .into_iter()
        .map(|(tool, count)| format!("{tool}×{count}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn render_failure(report: &RunReport) -> Option<String> {
    let text = report.failure_text.as_deref()?;
    if text.is_empty() {
        return None;
    }
    nonempty(explained_cut(text, FAILURE_CAP))
}

fn render_error_details(report: &RunReport) -> Option<String> {
    let denied: Vec<&LedgerSnap> = report
        .calls
        .iter()
        .filter(|call| call.error_code.as_deref() == Some("permission_denied"))
        .collect();
    if denied.is_empty() {
        return None;
    }
    let shown = denied.len().min(ERROR_DETAIL_LIMIT);
    let mut lines = Vec::new();
    for call in denied.iter().take(shown) {
        lines.push(format_error_detail(call));
    }
    let hidden = denied.len() - shown;
    if hidden > 0 {
        lines.push(format!("… {hidden} more errors not listed"));
    }
    nonempty(explained_cut(&lines.join("\n"), ERRORS_CAP))
}

fn format_error_detail(call: &LedgerSnap) -> String {
    let code = call.error_code.as_deref().unwrap_or("error");
    if call.summary.is_empty() {
        format!("#{} {} {code}", call.seq, call.tool)
    } else {
        format!("#{} {} {} {code}", call.seq, call.tool, call.summary)
    }
}

fn render_diagnostics(report: &RunReport) -> Option<String> {
    if report.diagnostics.is_empty() {
        return None;
    }
    let shown = report.diagnostics.len().min(DIAGNOSTIC_RENDER_CAP);
    let mut lines = vec!["diagnostics:".to_string()];
    for diagnostic in report.diagnostics.iter().take(shown) {
        lines.push(format_diagnostic(diagnostic));
    }
    let hidden = report.diagnostics.len() - shown + report.diagnostics_omitted;
    if hidden > 0 {
        lines.push(format!("… {hidden} more diagnostics not listed"));
    }
    nonempty(explained_cut(&lines.join("\n"), DIAGNOSTICS_CAP))
}

fn format_diagnostic(diagnostic: &Diagnostic) -> String {
    let path = detail_str(diagnostic, "path").unwrap_or_default();
    let mut line = if path.is_empty() {
        format!("- {}: {}", diagnostic.code, diagnostic.message)
    } else {
        format!("- {} {path}: {}", diagnostic.code, diagnostic.message)
    };
    if diagnostic.code == "lsp_error"
        && let Some(text) = detail_str(diagnostic, "text")
    {
        let text = prefix_utf8(&text, LSP_TEXT_CAP);
        if !text.is_empty() {
            line.push(' ');
            line.push_str(text);
        }
    }
    line
}

fn detail_str(diagnostic: &Diagnostic, key: &str) -> Option<String> {
    diagnostic
        .details
        .as_ref()
        .and_then(|details| details.get(key))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn render_ledger(report: &RunReport) -> Option<String> {
    if report.calls.is_empty() && report.calls_omitted == 0 {
        return None;
    }
    let rendered: Vec<String> = report.calls.iter().map(format_ledger_line).collect();
    let total = report.calls.len() + report.calls_omitted;
    let mut shown = rendered.len();
    loop {
        let body = ledger_body(report, &rendered, shown, total);
        if body.len() <= LEDGER_CAP || shown == 0 {
            if body.len() <= LEDGER_CAP {
                return Some(body);
            }
            return nonempty(explained_cut(&body, LEDGER_CAP));
        }
        shown -= 1;
    }
}

fn ledger_body(report: &RunReport, rendered: &[String], shown: usize, total: usize) -> String {
    let mut body = format!("calls ({shown} of {total} shown):");
    for line in rendered.iter().take(shown) {
        body.push('\n');
        body.push_str(line);
    }
    let hidden = total - shown;
    if hidden > 0 {
        body.push('\n');
        body.push_str(&ledger_omission(hidden, &report.calls[shown..], report.calls_omitted));
    }
    body
}

fn ledger_omission(hidden: usize, hidden_calls: &[LedgerSnap], ledger_omitted: usize) -> String {
    if ledger_omitted == 0 && !hidden_calls.is_empty() {
        format!(
            "… {hidden} more calls not listed ({})",
            tool_counts(hidden_calls)
        )
    } else {
        format!("… {hidden} more calls not listed")
    }
}

fn format_ledger_line(call: &LedgerSnap) -> String {
    if call.summary.is_empty() {
        format!("{}. {} {}", call.seq, call.tool, call.state)
    } else {
        format!(
            "{}. {} {} {}",
            call.seq, call.tool, call.summary, call.state
        )
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
    use crate::tool::outcome::Diagnostic;

    fn snap(seq: u64, tool: &str, summary: &str, state: &str, code: Option<&str>) -> LedgerSnap {
        LedgerSnap {
            seq,
            tool: tool.to_string(),
            summary: summary.to_string(),
            state: state.to_string(),
            error_code: code.map(str::to_string),
        }
    }

    fn report() -> RunReport {
        RunReport {
            logs: Vec::new(),
            value: None,
            failure_text: None,
            failure_kind: None,
            resource: None,
            timeout_ms: 120_000,
            output_limited: false,
            calls: Vec::new(),
            calls_omitted: 0,
            diagnostics: Vec::new(),
            diagnostics_omitted: 0,
        }
    }

    #[test]
    fn small_string_is_verbatim_and_has_no_json_copy() {
        let mut report = report();
        report.value = Some(Value::String("hello-small".into()));
        let text = render_model_view(&report);
        assert!(text.starts_with("run_code ok · 0 calls"), "{text}");
        assert!(text.contains("result:\nhello-small"), "{text}");
        assert!(!text.contains("@@run_code_data"), "{text}");
        assert!(!text.contains("\"hello-small\""), "{text}");
        assert!(text.len() <= RUN_CODE_VIEW_MAX_BYTES);
    }

    #[test]
    fn large_result_keeps_the_tail_and_states_the_omission() {
        let mut report = report();
        let mut body = "A".repeat(200_000);
        body.push_str("END_SENTINEL");
        report.value = Some(Value::String(body));
        let text = render_model_view(&report);
        assert!(text.starts_with("run_code ok"));
        assert!(text.len() <= RUN_CODE_VIEW_MAX_BYTES, "{}", text.len());
        assert!(text.contains("END_SENTINEL"), "{text}");
        assert!(text.contains("bytes omitted"), "{text}");
        assert!(!text.starts_with("[blob:"));
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
    fn section_caps_hold_for_oversized_inputs() {
        let mut report = report();
        report.failure_kind = Some(FailureKind::Timeout);
        report.failure_text = Some("x".repeat(20_000));
        report.calls = (0..64)
            .map(|seq| snap(seq, "read", &"p".repeat(400), "ok", None))
            .collect();
        report.diagnostics = (0..20)
            .map(|index| Diagnostic::warning(format!("code{index}"), "m".repeat(800)))
            .collect();
        report.logs = (0..40)
            .map(|index| ("info".into(), format!("{index}{}", "L".repeat(800))))
            .collect();
        report.value = Some(Value::String("R".repeat(80_000)));
        let text = render_model_view(&report);
        assert!(text.len() <= RUN_CODE_VIEW_MAX_BYTES, "{}", text.len());
        assert!(text.contains("bytes omitted"), "{text}");
        let failure_at = text.find(&"x".repeat(32)).expect("failure text");
        let after_failure = &text[failure_at..];
        let failure_end = after_failure
            .find("\n\n")
            .map(|index| failure_at + index)
            .unwrap_or(text.len());
        assert!(failure_end - failure_at <= FAILURE_CAP, "failure section grew");
        assert!(text.matches("\n- code").count() <= DIAGNOSTIC_RENDER_CAP);
    }

    #[test]
    fn success_header_counts_rejections_only_when_the_ledger_is_complete() {
        let mut report = report();
        report.calls = vec![
            snap(1, "read", "path=a", "ok", None),
            snap(2, "grep", "pattern=x", "error", Some("not_found")),
            snap(3, "grep", "pattern=y", "error", Some("not_found")),
        ];
        let text = render_model_view(&report);
        assert!(
            text.starts_with(
                "run_code ok · 3 calls (grep×2, read×1) · 2 rejected, handled by the program (not_found×2)"
            ),
            "{text}"
        );
        report.calls_omitted = 4;
        let partial = render_model_view(&report);
        assert!(partial.starts_with("run_code ok · 7 calls"), "{partial}");
        assert!(!partial.contains("rejected"), "{partial}");
        assert!(!partial.contains('×'), "{partial}");
    }

    #[test]
    fn ledger_line_does_not_print_the_error_code() {
        let mut report = report();
        report.calls = vec![snap(1, "write", "path=a.txt", "error", Some("permission_denied"))];
        let text = render_model_view(&report);
        let ledger = text
            .lines()
            .find(|line| line.starts_with("1. write"))
            .expect("ledger line");
        assert_eq!(ledger, "1. write path=a.txt error");
        assert!(text.contains("#1 write path=a.txt permission_denied"), "{text}");
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
    fn failure_header_omits_exact_counts_when_calls_were_dropped() {
        let mut report = report();
        report.failure_kind = Some(FailureKind::Timeout);
        report.failure_text = Some("run_code timed out after 120000ms".into());
        report.calls = vec![snap(1, "write", "path=a", "unsettled", None)];
        report.calls_omitted = 3;
        let text = render_model_view(&report);
        assert!(
            text.starts_with("run_code failed: timeout after 120000ms · 4 calls\n"),
            "{text}"
        );
        assert!(!text.contains("calls finished"), "{text}");
        assert!(!text.contains("were still running"), "{text}");

        report.calls_omitted = 0;
        let exact = render_model_view(&report);
        assert!(
            exact.starts_with(
                "run_code failed: timeout after 120000ms · 0/1 calls finished, 1 unsettled"
            ),
            "{exact}"
        );
        assert!(exact.contains("were still running"), "{exact}");
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

    #[test]
    fn omitted_cap_calls_do_not_invent_a_tool_breakdown() {
        let mut report = report();
        report.calls = vec![snap(1, "read", "path=a", "ok", None)];
        report.calls_omitted = 10;
        let text = render_model_view(&report);
        assert!(text.contains("… 10 more calls not listed"), "{text}");
        assert!(!text.contains("… 10 more calls not listed ("), "{text}");
    }
}

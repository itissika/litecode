//! Model-facing text for one `run_code` execution.
//!
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
    pub error_message: Option<String>,
    pub session_read: bool,
}

/// An uncaught `ToolError`, used for the header and the failure section.
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
    pub output_limited: bool,
    pub calls: Vec<LedgerSnap>,
    pub calls_omitted: usize,
    pub diagnostics: Vec<Diagnostic>,
    pub diagnostics_omitted: usize,
    pub tool_counts: BTreeMap<String, usize>,
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
    let unsettled = unsettled_total(report);
    if unsettled > 0 {
        parts.push(format!(
            "{unsettled} calls were still running when the program stopped; their side effects may still land. Re-read the files before retrying any write."
        ));
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

fn header_line(report: &RunReport) -> String {
    if let Some(error) = &report.tool_error {
        let mut line = format!(
            "run_code failed: uncaught ToolError {} from call #{} {} {}",
            error.code, error.seq, error.tool, error.summary
        );
        if let Some(rejected) = rejected_clause(report) {
            line.push_str(" · ");
            line.push_str(&rejected);
        }
        return line;
    }
    let total = call_total(report);
    let Some(kind) = report.failure_kind else {
        let mut line = format!("run_code ok · {total} calls");
        if let Some(counts) = header_tool_counts(report) {
            line.push_str(" (");
            line.push_str(&counts);
            line.push(')');
        }
        if let Some(rejected) = rejected_clause(report) {
            line.push_str(" · ");
            line.push_str(&rejected);
        }
        return line;
    };
    let label = if report.uncaught_type_error {
        "uncaught TypeError".to_string()
    } else {
        failure_label(kind, report)
    };
    let unsettled = unsettled_total(report);
    let finished = total.saturating_sub(unsettled);
    format!("run_code failed: {label} · {finished}/{total} calls finished, {unsettled} unsettled")
}

fn header_tool_counts(report: &RunReport) -> Option<String> {
    if !report.tool_counts.is_empty() {
        return Some(format_counts(&report.tool_counts));
    }
    if report.calls_omitted > 0 || report.calls.is_empty() {
        return None;
    }
    Some(tool_counts(&report.calls))
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

fn rejected_clause(report: &RunReport) -> Option<String> {
    if !report.error_counts.is_empty() {
        let total: usize = report.error_counts.values().copied().sum();
        return Some(rejection_sentence(
            report,
            total,
            &format_counts(&report.error_counts),
        ));
    }
    if report.calls_omitted > 0 {
        return None;
    }
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for call in &report.calls {
        let Some(code) = error_state_code(&call.state).or(call.error_code.as_deref()) else {
            continue;
        };
        if call.state.starts_with("error") {
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
    Some(rejection_sentence(report, total, &breakdown))
}

fn rejection_sentence(report: &RunReport, total: usize, breakdown: &str) -> String {
    if report.tool_error.is_some() {
        format!("{total} rejected ({breakdown}), 1 uncaught")
    } else {
        format!("{total} rejected ({breakdown}), handled by the program")
    }
}

fn error_state_code(state: &str) -> Option<&str> {
    state.strip_prefix("error:")
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

fn format_counts(counts: &BTreeMap<String, usize>) -> String {
    counts
        .iter()
        .map(|(name, count)| format!("{name}×{count}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn render_failure(report: &RunReport) -> Option<String> {
    if let Some(error) = &report.tool_error {
        let mut body = format!("{}: {}", error.code, error.message);
        if let Some(details) = &error.details {
            let encoded = serde_json::to_string(details).unwrap_or_default();
            if !encoded.is_empty() && encoded != "null" {
                body.push(' ');
                body.push_str(prefix_utf8(&encoded, 300));
            }
        }
        return nonempty(explained_cut(&body, FAILURE_CAP));
    }
    let mut parts = Vec::new();
    if let Some(message) = report
        .failure_text
        .as_deref()
        .filter(|text| !text.is_empty())
    {
        let message = prefix_utf8(message, 1024);
        if !message.is_empty() {
            parts.push(message.to_string());
        }
    }
    if report.failure_at_end {
        parts.push("at end of input".to_string());
    } else if let Some(location) = report.failure_location {
        parts.push(format!(
            "line {}, column {}",
            location.line, location.column
        ));
        if let Some(line) = source_line(&report.source, location.line) {
            let shown = prefix_utf8(line, 200);
            let caret_at = (location.column as usize)
                .saturating_sub(1)
                .min(shown.chars().count());
            parts.push(shown.to_string());
            parts.push(format!("{}^", " ".repeat(caret_at)));
        }
    }
    if shows_stack(report)
        && let Some(stack) = report.failure_stack.as_deref()
    {
        let stack = present_stack(stack);
        if !stack.is_empty() {
            parts.push(stack);
        }
    }
    if parts.is_empty() {
        return None;
    }
    nonempty(explained_cut(&parts.join("\n"), FAILURE_CAP))
}

fn shows_stack(report: &RunReport) -> bool {
    match report.failure_kind {
        Some(FailureKind::Exception) => true,
        Some(FailureKind::ResourceLimit) => report.resource == Some(ResourceKind::Stack),
        _ => false,
    }
}

fn source_line(source: &str, line: u32) -> Option<&str> {
    source.lines().nth(line.saturating_sub(1) as usize)
}

/// First 8 frames, with `program.js` line numbers shifted back off the wrapper.
fn present_stack(stack: &str) -> String {
    stack
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(adjust_program_frame)
        .take(8)
        .collect::<Vec<_>>()
        .join("\n")
}

fn adjust_program_frame(line: &str) -> Option<String> {
    let marker = "program.js:";
    let Some(start) = line.find(marker) else {
        return Some(line.to_string());
    };
    let digits_at = start + marker.len();
    let digits = line[digits_at..]
        .bytes()
        .take_while(u8::is_ascii_digit)
        .count();
    if digits == 0 {
        return Some(line.to_string());
    }
    let Ok(number) = line[digits_at..digits_at + digits].parse::<u32>() else {
        return Some(line.to_string());
    };
    let adjusted = number.saturating_sub(1);
    if adjusted == 0 {
        return None;
    }
    Some(format!(
        "{}{adjusted}{}",
        &line[..digits_at],
        &line[digits_at + digits..]
    ))
}

fn render_error_details(report: &RunReport) -> Option<String> {
    let uncaught = report.tool_error.as_ref().map(|error| error.seq);
    let mut picked: Vec<&LedgerSnap> = Vec::new();
    if let Some(seq) = uncaught
        && let Some(call) = report.calls.iter().find(|call| call.seq == seq)
    {
        picked.push(call);
    }
    for call in &report.calls {
        if Some(call.seq) == uncaught {
            continue;
        }
        if call.error_code.as_deref() == Some("permission_denied")
            || call.state == "error:permission_denied"
        {
            picked.push(call);
        }
    }
    let mut lines = Vec::new();
    if picked.is_empty() {
        if let Some(error) = &report.tool_error {
            let summary = if error.summary.is_empty() {
                String::new()
            } else {
                format!(" {}", error.summary)
            };
            lines.push(format!(
                "#{} {}{}: {}: {}",
                error.seq, error.tool, summary, error.code, error.message
            ));
        }
    } else {
        let shown = picked.len().min(ERROR_DETAIL_LIMIT);
        for call in picked.iter().take(shown) {
            lines.push(format_error_detail(call));
        }
        let hidden = picked.len() - shown;
        if hidden > 0 {
            lines.push(format!("… {hidden} more errors not listed"));
        }
    }
    if lines.is_empty() {
        return None;
    }
    nonempty(explained_cut(&lines.join("\n"), ERRORS_CAP))
}

fn format_error_detail(call: &LedgerSnap) -> String {
    let code = call
        .error_code
        .as_deref()
        .or_else(|| error_state_code(&call.state))
        .unwrap_or("error");
    let head = if call.summary.is_empty() {
        format!("#{} {}: {code}", call.seq, call.tool)
    } else {
        format!("#{} {} {}: {code}", call.seq, call.tool, call.summary)
    };
    match call.error_message.as_deref() {
        Some(message) if !message.is_empty() => format!("{head}: {message}"),
        _ => head,
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
    let total = call_total(report);
    if report.calls.is_empty() && total == 0 {
        return None;
    }
    let chosen = choose_ledger_lines(report, total);
    let rendered: Vec<String> = chosen.iter().map(|call| format_ledger_line(call)).collect();
    let body = ledger_body(report, &rendered, total, &chosen);
    if body.len() <= LEDGER_CAP {
        return Some(body);
    }
    nonempty(explained_cut(&body, LEDGER_CAP))
}

fn choose_ledger_lines<'a>(report: &'a RunReport, total: usize) -> Vec<&'a LedgerSnap> {
    // Reserve priority rows first, then fill from the head in seq order.
    let mut reserved = Vec::new();
    for call in &report.calls {
        if !is_priority_line(call) {
            continue;
        }
        reserved.push(call);
        if !ledger_fits(report, total, &reserved) {
            reserved.pop();
        }
    }
    let mut chosen = reserved;
    for call in &report.calls {
        if chosen.iter().any(|kept| kept.seq == call.seq) {
            continue;
        }
        chosen.push(call);
        if !ledger_fits(report, total, &chosen) {
            chosen.pop();
            break;
        }
    }
    chosen.sort_by_key(|call| call.seq);
    chosen
}

fn is_priority_line(call: &LedgerSnap) -> bool {
    call.state == "cancelled"
        || call.state == "unsettled"
        || call.state.starts_with("error:")
        || call.state.starts_with("ok(")
}

fn ledger_fits(report: &RunReport, total: usize, chosen: &[&LedgerSnap]) -> bool {
    let rendered: Vec<String> = chosen.iter().map(|call| format_ledger_line(call)).collect();
    ledger_body(report, &rendered, total, chosen).len() <= LEDGER_CAP
}

fn ledger_body(
    report: &RunReport,
    rendered: &[String],
    total: usize,
    chosen: &[&LedgerSnap],
) -> String {
    let shown = rendered.len();
    let mut body = format!("calls ({shown} of {total} shown):");
    for line in rendered {
        body.push('\n');
        body.push_str(line);
    }
    let hidden = total.saturating_sub(shown);
    if hidden > 0 {
        body.push('\n');
        body.push_str(&ledger_omission(report, hidden, chosen));
    }
    body
}

fn ledger_omission(report: &RunReport, hidden: usize, chosen: &[&LedgerSnap]) -> String {
    let mut line = if let Some(counts) = omitted_tool_counts(report, chosen) {
        format!("… {hidden} more calls not listed ({counts})")
    } else {
        format!("… {hidden} more calls not listed")
    };
    if omitted_session_read(report, chosen) {
        line.push(' ');
        line.push_str(RUN_CODE_SESSION_NOTE);
    }
    line
}

fn omitted_tool_counts(report: &RunReport, chosen: &[&LedgerSnap]) -> Option<String> {
    if !report.tool_counts.is_empty() {
        let mut counts = report.tool_counts.clone();
        for call in chosen {
            if let Some(count) = counts.get_mut(&call.tool) {
                *count = count.saturating_sub(1);
            }
        }
        counts.retain(|_, count| *count > 0);
        if counts.is_empty() {
            return None;
        }
        return Some(format_counts(&counts));
    }
    if report.calls_omitted > 0 {
        return None;
    }
    let shown: std::collections::HashSet<u64> = chosen.iter().map(|call| call.seq).collect();
    let hidden: Vec<&LedgerSnap> = report
        .calls
        .iter()
        .filter(|call| !shown.contains(&call.seq))
        .collect();
    if hidden.is_empty() {
        return None;
    }
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for call in hidden {
        *counts.entry(call.tool.as_str()).or_default() += 1;
    }
    Some(
        counts
            .into_iter()
            .map(|(tool, count)| format!("{tool}×{count}"))
            .collect::<Vec<_>>()
            .join(", "),
    )
}

fn omitted_session_read(report: &RunReport, chosen: &[&LedgerSnap]) -> bool {
    if report.session_read_hidden {
        return true;
    }
    let shown: std::collections::HashSet<u64> = chosen.iter().map(|call| call.seq).collect();
    report
        .calls
        .iter()
        .any(|call| call.session_read && !shown.contains(&call.seq))
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
            error_message: None,
            session_read: false,
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
            tool_counts: BTreeMap::new(),
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
        assert!(
            failure_end - failure_at <= FAILURE_CAP,
            "failure section grew"
        );
        assert!(text.matches("\n- code").count() <= DIAGNOSTIC_RENDER_CAP);
    }

    #[test]
    fn wrapper_frame_on_line_zero_is_dropped() {
        let stack = "at program.js:1:1\n    at boom (program.js:3:5)";
        assert_eq!(present_stack(stack), "    at boom (program.js:2:5)");
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
                "run_code ok · 3 calls (grep×2, read×1) · 2 rejected (not_found×2), handled by the program"
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
    fn ledger_line_prints_the_error_code_and_detail_message() {
        let mut report = report();
        let mut call = snap(
            1,
            "write",
            "path=a.txt",
            "error:permission_denied",
            Some("permission_denied"),
        );
        call.error_message = Some("denied".into());
        report.calls = vec![call];
        let text = render_model_view(&report);
        let ledger = text
            .lines()
            .find(|line| line.starts_with("1. write"))
            .expect("ledger line");
        assert_eq!(ledger, "1. write path=a.txt error:permission_denied");
        assert!(
            text.contains("#1 write path=a.txt: permission_denied: denied"),
            "{text}"
        );
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
    fn failure_header_counts_dropped_calls() {
        let mut report = report();
        report.failure_kind = Some(FailureKind::Timeout);
        report.failure_text = Some("run_code timed out after 120000ms".into());
        report.calls = vec![snap(1, "write", "path=a", "unsettled", None)];
        report.calls_omitted = 3;
        report.call_total = Some(4);
        report.unsettled_total = Some(1);
        let text = render_model_view(&report);
        assert!(
            text.starts_with(
                "run_code failed: timeout after 120000ms · 3/4 calls finished, 1 unsettled"
            ),
            "{text}"
        );
        assert!(text.contains("1 calls were still running"), "{text}");
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

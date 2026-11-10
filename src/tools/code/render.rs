//! Model-facing text for one `run_code` execution.
//!
//! The view is a header, then **one ordered stream**: console lines and the
//! program's return value in the order they happened, the value last. There is
//! no `logs:` half and no `result:` half, and exactly one token budget
//! ([`RUN_CODE_VIEW_TOKENS`]) covers the whole stream. On failure the header is
//! followed by a brief of the calls that already settled, then the same stream.
//!
//! Uniformly shaped object arrays render as `[N]{a,b}:` tables (columns in
//! first-seen key order, a key a row omits renders as an empty cell); other
//! values render as compact JSON. A log line or value string that is itself one
//! JSON value is lexically compacted; other strings stay verbatim. Structured
//! values degrade structurally, while over-budget strings keep both a head and
//! a tail (WP-210 C). [`RUN_CODE_VIEW_MAX_BYTES`] remains the hard byte
//! fallback. Cuts land on UTF-8 boundaries, and every omission marker says
//! whether the token budget or the byte cap caused it. This module does not
//! emit a JSON copy of the run.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::ptc::{FailureKind, ResourceKind, SourceLocation};
use crate::session::transcript_file::RUN_CODE_SESSION_NOTE;
use crate::types::utf8::prefix_utf8;

/// Hard cap for the model-facing `run_code` view, including omission notes.
pub const RUN_CODE_VIEW_MAX_BYTES: usize = 65_536;
/// The one cl100k budget for the whole output stream. Omission markers count
/// against it. There is deliberately no second budget: the return value is just
/// the last entry and competes with the console lines for the same tokens.
pub const RUN_CODE_VIEW_TOKENS: usize = 10_000;

const HEADER_CAP: usize = 1024;
const HEAD_MESSAGE_CAP: usize = 240;

/// One nested call as counted for the head. Kept for host/tests; not rendered
/// as a ledger section in the default model view.
pub(super) struct LedgerSnap {
    pub seq: u64,
    pub tool: String,
    pub summary: String,
    pub state: String,
    pub error_code: Option<String>,
    pub session_read: bool,
    /// One-line result digest (WP-210 A); `None` while unsettled.
    pub brief: Option<String>,
}

/// An uncaught `ToolError`, used for the header failure tag.
pub(super) struct UncaughtToolError {
    pub seq: u64,
    pub tool: String,
    pub code: String,
    pub message: String,
    pub summary: String,
}

/// A one-line summary of one call's facts.
///
/// The call is the anchor, not a line of output: the program may return only a
/// slice of a result, or none of it, and the facts are still true. The bridge
/// computes this when the call settles.
pub(super) struct CallReminder {
    /// Call sequence number, matching the numbering used elsewhere in the view.
    pub seq: u64,
    /// Dotted tool name as the program sees it (\`file.read\`).
    pub tool: String,
    /// Rendered by this module from the call's result fields.
    pub text: String,
}

/// One reminder takes one line, at most this many characters, and is charged to
/// the same token budget as the stream.
const REMINDER_MAX_CHARS: usize = 120;
/// Marks a renderer-generated summary: without it a reminder reads exactly like
/// the program printing a bare string, or like a returned value.
const REMINDER_MARK: &str = "↳ ";

/// One entry of the stream as the renderer sees it.
pub(super) enum EntryView {
    Text { level: String, text: String },
    Value(Value),
}

/// Facts the renderer needs. Built from the in-memory execution report.
pub(super) struct RunReport {
    /// Console text and, last, the return value — in the order they happened.
    pub entries: Vec<EntryView>,
    /// One-line summaries, one per call that has facts; see [`CallReminder`].
    pub reminders: Vec<CallReminder>,
    pub failure_text: Option<String>,
    pub failure_kind: Option<FailureKind>,
    pub resource: Option<ResourceKind>,
    pub timeout_ms: u64,
    pub duration_ms: u64,
    pub calls: Vec<LedgerSnap>,
    pub calls_omitted: usize,
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
    /// `coerced non-finite number at ... to null` head line.
    pub coerced_non_finite: Option<String>,
}

pub(super) fn render_model_view(report: &RunReport) -> String {
    let header = render_header_block(report);
    // One budget covers everything the model sees: header, manifest, stream.
    let mut budget = RUN_CODE_VIEW_TOKENS.saturating_sub(count_tokens(&header));
    let manifest = render_completed_calls(report);
    if let Some(manifest) = &manifest {
        budget = budget.saturating_sub(count_tokens(manifest));
    }
    let output_limited = report.failure_kind == Some(FailureKind::OutputLimit);
    // Reminders describe calls, so they follow the stream and share its budget.
    let reminders = render_reminder_lines(&report.reminders, budget / 4);
    let stream = render_stream(
        &report.entries,
        budget.saturating_sub(count_tokens(&reminders)),
        output_limited,
    );
    let mut sections = vec![header];
    sections.extend(manifest);
    if !stream.is_empty() {
        sections.push(stream);
    }
    if !reminders.is_empty() {
        sections.push(reminders);
    }
    let text = sections.join("\n\n");
    if text.len() <= RUN_CODE_VIEW_MAX_BYTES {
        return text;
    }
    // Byte-heavy content can still trip the cap after the token budget passed.
    byte_cut(&text, RUN_CODE_VIEW_MAX_BYTES)
}


/// Program frames shown under a runtime exception (WP-210 B/Q3).
const STACK_FRAMES: usize = 5;

/// Appended to the last stream line when the run stopped on the output cap.
const OUTPUT_LIMIT_MARK: &str = " [output limit reached]";


/// Failed run: the calls that already settled, one line each, no section title.
///
/// A program that stopped at call N must still see what 1..N-1 did — a write that
/// already landed is not undone by the failure. Compressed to tool, outcome and a
/// one-line digest.
fn render_completed_calls(report: &RunReport) -> Option<String> {
    if report.failure_kind.is_none() || did_not_start(report) {
        return None;
    }
    let settled: Vec<&LedgerSnap> = report
        .calls
        .iter()
        .filter(|call| call.brief.is_some())
        .collect();
    if settled.is_empty() {
        return None;
    }
    let mut lines = Vec::with_capacity(settled.len());
    for call in settled {
        let status = if call.state.starts_with("error") {
            match &call.error_code {
                Some(code) => format!("failed({code})"),
                None => "failed".to_string(),
            }
        } else if call.state == "cancelled" {
            "cancelled".to_string()
        } else {
            "ok".to_string()
        };
        let brief = call.brief.as_deref().unwrap_or("");
        let summary = if call.summary.is_empty() {
            String::new()
        } else {
            format!(" {}", call.summary)
        };
        lines.push(format!(
            "#{} {}{summary} → {status}: {brief}",
            call.seq, call.tool
        ));
    }
    let hidden = call_total(report).saturating_sub(report.calls.len());
    if hidden > 0 {
        lines.push(format!("… {hidden} more"));
    }
    Some(lines.join("\n"))
}

/// Final byte-cap cut: both ends survive and the middle says why.
fn byte_cut(text: &str, cap: usize) -> String {
    if text.len() <= cap {
        return text.to_string();
    }
    let head = crate::types::utf8::prefix_utf8(text, cap * 2 / 3).to_string();
    let tail = crate::types::utf8::suffix_utf8(text, cap / 4).to_string();
    let omitted = text.len().saturating_sub(head.len() + tail.len());
    format!("{head}\n… [truncated by byte cap: {omitted} bytes omitted] …\n{tail}")
}

/// Summaries for one settled call, keyed by the tool that produced it.
///
/// This is the only place that writes reminder wording: tools produce fields,
/// and the renderer turns a (tool, field) pair into one line. The tool name is
/// the identity — a result shape could belong to any of several tools.
pub(super) fn reminders_for_call(tool: &str, value: &Value) -> Vec<String> {
    let Some(object) = value.as_object() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    match tool {
        "file.read" => read_reminders(object, &mut out),
        "file.glob" => glob_reminders(object, &mut out),
        "file.grep" => grep_reminders(object, &mut out),
        tool if tool.starts_with("lsp.") => lsp_reminders(tool, object, &mut out),
        _ => {}
    }
    out
}

/// LSP results carry an index state; `partial` and `unknown` are different
/// facts and must not collapse into one sentence.
fn lsp_reminders(
    tool: &str,
    object: &serde_json::Map<String, Value>,
    out: &mut Vec<String>,
) {
    let subject = match tool {
        "lsp.hover" => "hover information",
        "lsp.diagnostics" => "diagnostics",
        "lsp.document_symbol" | "lsp.workspace_symbol" => "symbols",
        _ => "locations",
    };
    let state = object.get("state").and_then(Value::as_str).unwrap_or("");
    let empty = match tool {
        "lsp.hover" => object
            .get("markdown")
            .and_then(Value::as_str)
            .is_none_or(|markdown| markdown.trim().is_empty()),
        "lsp.diagnostics" => object
            .get("items")
            .and_then(Value::as_array)
            .is_none_or(Vec::is_empty),
        "lsp.document_symbol" | "lsp.workspace_symbol" => object
            .get("symbols")
            .and_then(Value::as_array)
            .is_none_or(Vec::is_empty),
        _ => object
            .get("locations")
            .and_then(Value::as_array)
            .is_none_or(Vec::is_empty),
    };
    let line = match (empty, state) {
        (true, "ok") => format!("no {subject} reported; the index is ready, so this is a real miss"),
        (true, "partial") => format!(
            "no {subject} reported; the index is still building, so this is not a confirmed miss"
        ),
        (true, _) => format!(
            "no {subject} reported; the index state is unknown, so this is not a confirmed miss"
        ),
        (false, "partial") => "results may be incomplete; the index is still building".to_string(),
        (false, "unknown") => "results may be incomplete; the index state is unknown".to_string(),
        (false, _) => return,
    };
    out.push(line);
}

fn read_reminders(object: &serde_json::Map<String, Value>, out: &mut Vec<String>) {
    let start_line = object.get("start_line").and_then(Value::as_u64).unwrap_or(0);
    let total_lines = object
        .get("total_lines")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let lines = object.get("lines").and_then(Value::as_array);
    let shown = lines.map_or(0, Vec::len) as u64;
    let skipped = object
        .get("skipped_in_context")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    if skipped > 0 {
        out.push(format!(
            "{skipped} lines were left out because they are already in your context"
        ));
    }
    if lines.is_some_and(Vec::is_empty) && start_line > total_lines {
        out.push(format!(
            "start_line {start_line} is past the end; the file has {total_lines} lines"
        ));
    } else if object.get("has_more").and_then(Value::as_bool) == Some(true) {
        // Every line the page covered is either in the list or counted as
        // skipped, so the window end is exact even for a fully hidden page.
        let covered = shown + skipped;
        let end = start_line + covered.saturating_sub(1);
        out.push(format!(
            "showing lines {start_line}-{end} of {total_lines}; continue with start_line {}",
            start_line + covered
        ));
    }
    if let Some(capped) = object
        .get("lines_capped")
        .and_then(Value::as_array)
        .filter(|capped| !capped.is_empty())
    {
        let mut numbers: Vec<String> = capped.iter().take(5).map(compact_number).collect();
        let more = capped.len().saturating_sub(5);
        if more > 0 {
            numbers.push(format!("+{more} more"));
        }
        out.push(format!(
            "lines {} shortened to fit the line cap",
            numbers.join(", ")
        ));
    }
    out.extend(clamp_reminders("read", object));
}

fn grep_reminders(object: &serde_json::Map<String, Value>, out: &mut Vec<String>) {
    if let Some(reason) = object.get("empty_reason").and_then(Value::as_str) {
        let text = match reason {
            "glob_no_files" => "no file matched the glob filter",
            "file_unreadable" => "the file could not be read as text",
            "binary" => "the file is binary; grep only searches text files",
            "hidden_by_filters" => "the searched path is excluded by default filters",
            "no_match" => "the pattern was not found",
            "no_files" => "nothing to search under this path",
            other => other,
        };
        out.push(text.to_string());
    }
    let total = object.get("total_hits").and_then(Value::as_u64);
    let shown = object.get("hits").and_then(Value::as_array).map_or(0, Vec::len);
    match (object.get("at_limit_cap"), total) {
        (Some(Value::Bool(true)), Some(total)) => out.push(format!(
            "stopped at the hit limit; showing {shown} of {total}"
        )),
        (_, Some(total)) if total > shown as u64 => {
            out.push(format!("showing {shown} of {total} matches"));
        }
        _ => {}
    }
    if object.get("lines_truncated").and_then(Value::as_bool) == Some(true) {
        out.push("long lines were shortened to fit the line cap".to_string());
    }
    if object.get("context_omitted").and_then(Value::as_bool) == Some(true) {
        out.push("context lines were omitted to fit the budget".to_string());
    }
    if object.get("output_capped").and_then(Value::as_bool) == Some(true) {
        out.push("output was capped; narrow the pattern or path".to_string());
    }
    out.extend(clamp_reminders("grep", object));
}

fn glob_reminders(object: &serde_json::Map<String, Value>, out: &mut Vec<String>) {
    let total = object.get("total").and_then(Value::as_u64).unwrap_or(0);
    let shown = object
        .get("paths")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    if object.get("truncated").and_then(Value::as_bool) == Some(true) {
        out.push(format!(
            "showing first {shown} of {total} paths; raise limit to see the rest"
        ));
    }
    if let Some(reason) = object.get("empty_reason").and_then(Value::as_str) {
        let text = match reason {
            "hidden_by_filters" => {
                "the searched path is excluded by default filters; an empty page is not \"no matches\""
            }
            "no_match" => "no path matched the pattern",
            other => other,
        };
        out.push(text.to_string());
    }
    if object.get("outside_workspace").and_then(Value::as_bool) == Some(true) {
        out.push("paths outside the workspace are absolute".to_string());
    }
    out.extend(clamp_reminders("glob", object));
}

fn clamp_reminders(tool: &str, object: &serde_json::Map<String, Value>) -> Vec<String> {
    let Some(clamped) = object.get("clamped").and_then(Value::as_array) else {
        return Vec::new();
    };
    clamped
        .iter()
        .filter_map(|clamp| {
            let parameter = clamp.get("parameter")?.as_str()?;
            let requested = clamp.get("requested")?;
            let applied = clamp.get("applied")?;
            Some(format!(
                "{parameter} {} used instead of {}",
                compact_number(applied),
                compact_number(requested)
            ))
        })
        .collect()
}

fn compact_number(value: &Value) -> String {
    match value {
        Value::Number(number) => number.to_string(),
        other => other.to_string(),
    }
}

/// Render one entry as it appears in the stream.
fn entry_line(entry: &EntryView) -> String {
    match entry {
        EntryView::Text { level, text } => log_line(level, text),
        EntryView::Value(value) => render_result_body(value),
    }
}

/// Render the whole stream within one token budget.
///
/// Fits: everything, in order. Over budget: every entry gets a fair share of the
/// same budget (water-filling, so one long line cannot starve the rest). Too many
/// entries for a meaningful share: keep the head and say how many are gone. The
/// return value is not special-cased; it is the last entry and is cut like one.
fn render_stream(entries: &[EntryView], budget: usize, output_limited: bool) -> String {
    if entries.is_empty() || budget == 0 {
        return String::new();
    }
    let mut lines: Vec<String> = entries.iter().map(entry_line).collect();
    if output_limited && let Some(last) = lines.last_mut() {
        last.push_str(OUTPUT_LIMIT_MARK);
    }
    let costs: Vec<usize> = lines.iter().map(|line| count_tokens(line)).collect();
    let total: usize = costs.iter().sum();
    if total <= budget {
        return lines.join("
");
    }
    if let Some(allowances) = water_fill_allowances(&costs, budget) {
        let mut out = Vec::with_capacity(entries.len());
        for (index, entry) in entries.iter().enumerate() {
            out.push(entry_within(entry, &lines[index], allowances[index]));
        }
        let text = out.join("
");
        if count_tokens(&text) <= budget {
            return text;
        }
    }
    // Not even a minimum share per entry: keep the head of the stream.
    let reserve = count_tokens("… 000 entries omitted (token budget)") + 1;
    let mut kept = 0usize;
    let mut used = 0usize;
    for cost in &costs {
        if used + cost + reserve > budget {
            break;
        }
        used += cost;
        kept += 1;
    }
    // BPE counts are not additive across line joins, so trim until it fits.
    loop {
        let mut out: Vec<String> = lines[..kept].to_vec();
        let omitted = lines.len() - kept;
        if omitted > 0 {
            out.push(format!("… {omitted} entries omitted (token budget)"));
        }
        let text = out.join("
");
        if count_tokens(&text) <= budget || kept == 0 {
            return text;
        }
        kept -= 1;
    }
}

/// One line per call, in call order, charged to the same budget.
fn render_reminder_lines(reminders: &[CallReminder], budget: usize) -> String {
    let mut lines: Vec<String> = Vec::with_capacity(reminders.len());
    let mut used = 0usize;
    for reminder in reminders {
        let line = reminder_line(&format!(
            "#{} {}: {}",
            reminder.seq, reminder.tool, reminder.text
        ));
        let cost = count_tokens(&line);
        if used + cost > budget && !lines.is_empty() {
            break;
        }
        used += cost;
        lines.push(line);
    }
    lines.join("\n")
}

fn reminder_line(text: &str) -> String {
    let one_line = text.lines().next().unwrap_or("").trim();
    let room = REMINDER_MAX_CHARS.saturating_sub(REMINDER_MARK.chars().count());
    format!("{REMINDER_MARK}{}", prefix_utf8(one_line, room))
}

/// Render one entry inside its share of the budget.
fn entry_within(entry: &EntryView, line: &str, allowance: usize) -> String {
    let cost = count_tokens(line);
    match entry {
        EntryView::Text { .. } => {
            if cost <= allowance {
                line.to_string()
            } else {
                truncate_string_result(line, allowance, "", cost)
            }
        }
        EntryView::Value(value) => {
            let body = budget_value(value, allowance);
            if body.is_empty() {
                "… [value omitted (token budget)]".to_string()
            } else {
                body
            }
        }
    }
}


fn count_tokens(text: &str) -> usize {
    crate::session::estimate::count_text_tokens(text)
}

fn log_line(level: &str, message: &str) -> String {
    let message = compact_json_text(message).unwrap_or_else(|| message.to_string());
    format!("[{level}] {message}")
}

/// Least tokens one entry must get before fair sharing is worth it; below this
/// a truncated entry is almost all marker.
const MIN_LOG_ENTRY_TOKENS: usize = 64;

/// Per-entry token allowances covering the whole section: entries at or under
/// the current share keep everything, the rest split the leftover evenly
/// (water-filling). None when not every entry can get MIN_LOG_ENTRY_TOKENS;
/// that many entries are better served by the head prefix plus one omission
/// marker.
fn water_fill_allowances(costs: &[usize], budget: usize) -> Option<Vec<usize>> {
    if costs.is_empty() || budget == 0 {
        return None;
    }
    let mut share = budget / costs.len();
    if share < MIN_LOG_ENTRY_TOKENS {
        return None;
    }
    let mut fixed = vec![false; costs.len()];
    loop {
        let mut used = 0usize;
        let mut pending = 0usize;
        for (index, cost) in costs.iter().enumerate() {
            if fixed[index] {
                used += cost;
            } else if *cost <= share {
                fixed[index] = true;
                used += cost;
            } else {
                pending += 1;
            }
        }
        if pending == 0 {
            return Some(costs.to_vec());
        }
        let next = budget.saturating_sub(used) / pending;
        if next < MIN_LOG_ENTRY_TOKENS {
            return None;
        }
        if next == share {
            let mut allowances = costs.to_vec();
            for (index, allowance) in allowances.iter_mut().enumerate() {
                if !fixed[index] {
                    *allowance = share;
                }
            }
            return Some(allowances);
        }
        share = next;
    }
}

fn format_ktok(tokens: usize) -> String {
    if tokens >= 1000 {
        format!("{:.1}k", tokens as f64 / 1000.0)
    } else {
        tokens.to_string()
    }
}

fn over_budget_notice(total: usize, budget: usize) -> String {
    format!(
        "result {} tok exceeds {}; truncated by token budget — return summary/counts/paths",
        format_ktok(total),
        format_ktok(budget),
    )
}

fn fits(text: &str, budget: usize) -> bool {
    text.len() <= budget || count_tokens(text) <= budget
}

/// Render `value` within `budget` cl100k tokens, counting the over-budget
/// notice and omission markers (WP-210 C).
///
/// Strings are text and keep both ends: a token-budgeted head, a middle-cut
/// marker, and a token-budgeted tail. Structured values degrade structurally
/// and stay valid JSON / whole table rows:
/// 1. every array keeps its first K items, the rest become `"… N more"`
///    (tables: one `… N more rows` line);
/// 2. strings longer than S chars become `prefix…(N chars)`;
/// 3. containers deeper than D become `"{… N keys}"` / `"[… N items]"`.
///
/// Content comes before structure: cutting a long string keeps the key and the
/// useful prefix, while collapsing a container would throw all of it away to
/// save the same bytes. Each knob is the largest value that fits (binary
/// search).
fn budget_value(value: &Value, budget: usize) -> String {
    let body = render_result_body(value);
    if fits(&body, budget) {
        return body;
    }
    if budget == 0 {
        return String::new();
    }
    let total = count_tokens(&body);
    let notice = over_budget_notice(total, budget);
    if let Value::String(_) = value {
        return truncate_string_result(&body, budget, &notice, total);
    }
    let render = |shape: &Shape| render_result_body(&degrade(value, shape, 0));
    let fits_with_notice = |candidate: &str| fits(&format!("{notice}\n{candidate}"), budget);
    let max_len = longest_array(value);
    let mut shape = Shape {
        keep: max_len,
        depth: DEGRADE_MAX_DEPTH,
        chars: usize::MAX,
    };
    // 1. array length
    let (mut lo, mut hi) = (0usize, max_len);
    while lo < hi {
        let mid = (lo + hi + 1) / 2;
        shape.keep = mid;
        if fits_with_notice(&render(&shape)) {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    shape.keep = lo.max(1);
    if fits_with_notice(&render(&shape)) {
        return format!("{notice}\n{}", render(&shape));
    }
    // 2. string length. Try content-preserving cuts before any container
    //    collapse: a shallow object of long file texts otherwise "fits" by
    //    replacing every value with `{… N keys}`, which drops the whole payload.
    let (mut lo, mut hi) = (0usize, longest_string(value));
    while lo < hi {
        let mid = (lo + hi + 1) / 2;
        shape.chars = mid;
        if fits_with_notice(&render(&shape)) {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    shape.chars = lo;
    if fits_with_notice(&render(&shape)) {
        return format!("{notice}\n{}", render(&shape));
    }
    // 3. depth. Even empty strings are too much; collapse deep containers.
    let max_depth = depth_of(value);
    let (mut lo, mut hi) = (1usize, max_depth.clamp(1, DEGRADE_MAX_DEPTH));
    while lo < hi {
        let mid = (lo + hi + 1) / 2;
        shape.depth = mid;
        if fits_with_notice(&render(&shape)) {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    shape.depth = lo;
    if fits_with_notice(&render(&shape)) {
        return format!("{notice}\n{}", render(&shape));
    }
    // Still over (e.g. a huge flat object): whole leading lines only, with an
    // explicit token-budget marker so this cut is distinguishable from the
    // 64KB byte fallback.
    let marker = "… [truncated by token budget]";
    let reserve = count_tokens(&notice) + count_tokens(marker) + 1;
    let mut cap = budget.saturating_sub(reserve);
    loop {
        let kept = token_prefix(&body, cap);
        let rendered = format!("{notice}\n{kept}\n{marker}");
        if fits(&rendered, budget) {
            return rendered;
        }
        if cap == 0 {
            break;
        }
        let over = count_tokens(&rendered).saturating_sub(budget).max(1);
        cap = cap.saturating_sub(over);
    }
    let compact = format!("{notice}\n{marker}");
    if fits(&compact, budget) {
        return compact;
    }
    if fits(&notice, budget) {
        return notice;
    }
    String::new()
}

fn truncate_string_result(body: &str, budget: usize, notice: &str, total: usize) -> String {
    let prefix = if notice.is_empty() {
        String::new()
    } else {
        format!("{notice}\n")
    };
    let mut content_budget = budget
        .saturating_sub(count_tokens(notice))
        .saturating_sub(32);
    loop {
        let head_budget = content_budget * 2 / 3;
        let head = token_prefix(body, head_budget);
        let head_tokens = count_tokens(head);
        let tail_budget = content_budget.saturating_sub(head_tokens);
        let tail = if head.len() < body.len() {
            token_suffix(&body[head.len()..], tail_budget)
        } else {
            ""
        };
        let kept_tokens = head_tokens + count_tokens(tail);
        let omitted_tokens = total.saturating_sub(kept_tokens);
        let marker = format!(
            "\n… [truncated by token budget: {kept_tokens} of {total} tok kept; {omitted_tokens} tok omitted from the middle] …\n"
        );
        let rendered = if tail.is_empty() {
            format!("{prefix}{head}{marker}")
        } else {
            format!("{prefix}{head}{marker}{tail}")
        };
        if count_tokens(&rendered) <= budget {
            return rendered;
        }
        if content_budget == 0 {
            break;
        }
        let over = count_tokens(&rendered).saturating_sub(budget).max(16);
        content_budget = content_budget.saturating_sub(over);
    }
    let compact = format!("{prefix}… [truncated by token budget: 0 of {total} tok kept]");
    if fits(&compact, budget) {
        compact
    } else {
        String::new()
    }
}

const DEGRADE_MAX_DEPTH: usize = 256;

struct Shape {
    keep: usize,
    depth: usize,
    chars: usize,
}

fn more_marker(count: usize) -> Value {
    Value::String(format!("… {count} more"))
}

fn is_more_marker(value: &Value) -> Option<usize> {
    let text = value.as_str()?;
    text.strip_prefix("… ")?.strip_suffix(" more")?.parse().ok()
}

/// Apply `shape` to a copy of `value`. Recursion is bounded by
/// [`DEGRADE_MAX_DEPTH`]: anything deeper collapses to a placeholder.
fn degrade(value: &Value, shape: &Shape, depth: usize) -> Value {
    match value {
        Value::Array(items) => {
            if depth >= shape.depth && !items.is_empty() {
                return Value::String(format!("[…{} items]", items.len()));
            }
            let keep = items.len().min(shape.keep);
            let mut out: Vec<Value> = items[..keep]
                .iter()
                .map(|item| degrade(item, shape, depth + 1))
                .collect();
            if keep < items.len() {
                out.push(more_marker(items.len() - keep));
            }
            Value::Array(out)
        }
        Value::Object(map) => {
            if depth >= shape.depth && !map.is_empty() {
                return Value::String(format!("{{…{} keys}}", map.len()));
            }
            Value::Object(
                map.iter()
                    .map(|(key, child)| (key.clone(), degrade(child, shape, depth + 1)))
                    .collect(),
            )
        }
        Value::String(text) => {
            let count = text.chars().count();
            if count > shape.chars {
                let cut: String = text.chars().take(shape.chars).collect();
                Value::String(format!("{cut}…({count} chars)"))
            } else {
                value.clone()
            }
        }
        other => other.clone(),
    }
}

fn walk_stats(value: &Value) -> (usize, usize, usize) {
    // (longest array, max depth, longest string in chars), iterative.
    let (mut array, mut depth, mut string) = (0usize, 0usize, 0usize);
    let mut stack = vec![(value, 1usize)];
    while let Some((node, level)) = stack.pop() {
        depth = depth.max(level);
        match node {
            Value::Array(items) => {
                array = array.max(items.len());
                stack.extend(items.iter().map(|item| (item, level + 1)));
            }
            Value::Object(map) => stack.extend(map.values().map(|child| (child, level + 1))),
            Value::String(text) => string = string.max(text.chars().count()),
            _ => {}
        }
    }
    (array, depth, string)
}

fn longest_array(value: &Value) -> usize {
    walk_stats(value).0
}

fn depth_of(value: &Value) -> usize {
    walk_stats(value).1
}

fn longest_string(value: &Value) -> usize {
    walk_stats(value).2
}

/// Longest prefix within `budget` tokens. Cuts at a line end when at least one
/// whole line fits (tables never lose half a row), otherwise at a char boundary.
fn token_prefix(body: &str, budget: usize) -> &str {
    let line_ends: Vec<usize> = body
        .char_indices()
        .filter(|(_, c)| *c == '\n')
        .map(|(i, _)| i)
        .collect();
    // Largest k such that the first k lines fit.
    let (mut lo, mut hi) = (0usize, line_ends.len());
    while lo < hi {
        let mid = (lo + hi + 1) / 2;
        if count_tokens(&body[..line_ends[mid - 1]]) <= budget {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    if lo > 0 {
        return &body[..line_ends[lo - 1]];
    }
    let first = &body[..line_ends.first().copied().unwrap_or(body.len())];
    let bounds: Vec<usize> = first
        .char_indices()
        .map(|(i, _)| i)
        .chain([first.len()])
        .collect();
    let (mut lo, mut hi) = (0usize, bounds.len() - 1);
    while lo < hi {
        let mid = (lo + hi + 1) / 2;
        if count_tokens(&first[..bounds[mid]]) <= budget {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    &first[..bounds[lo]]
}

/// Longest suffix of `text` within `budget` tokens, cut at a char boundary.
fn token_suffix(text: &str, budget: usize) -> &str {
    if text.len() <= budget || count_tokens(text) <= budget {
        return text;
    }
    let bounds: Vec<usize> = text
        .char_indices()
        .map(|(i, _)| i)
        .chain([text.len()])
        .collect();
    // Smallest start index whose suffix fits.
    let (mut lo, mut hi) = (0usize, bounds.len() - 1);
    while lo < hi {
        let mid = (lo + hi) / 2;
        if count_tokens(&text[bounds[mid]..]) <= budget {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    &text[bounds[lo]..]
}

fn render_result_body(value: &Value) -> String {
    match value {
        Value::String(text) => compact_json_text(text).unwrap_or_else(|| text.clone()),
        Value::Array(items) => match table_columns(items) {
            Some(columns) => render_table("", items, &columns),
            None => compact_json(value),
        },
        Value::Object(map) => {
            let mut rest = serde_json::Map::new();
            let mut tables = Vec::new();
            for (key, v) in map {
                match v
                    .as_array()
                    .and_then(|items| table_columns(items).map(|columns| (items, columns)))
                {
                    Some((items, columns)) => {
                        tables.push(render_table(&table_key(key), items, &columns))
                    }
                    None => {
                        rest.insert(key.clone(), v.clone());
                    }
                }
            }
            // No table-shaped member: the JSON line is the whole object.
            if tables.is_empty() {
                return compact_json(value);
            }
            let mut parts = Vec::new();
            if !rest.is_empty() {
                parts.push(compact_json(&Value::Object(rest)));
            }
            parts.extend(tables);
            parts.join("\n")
        }
        other => compact_json(other),
    }
}

fn compact_json(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| value.to_string())
}

/// A string longer than this is not parsed just to compact JSON text.
const MAX_COMPACT_JSON_TEXT_BYTES: usize = super::MAX_CODE_OUTPUT_BYTES;

/// Lexically compact a string that is one JSON value.
///
/// Validation uses `serde_json` only to prove the text is a single JSON value;
/// the output comes from a scan that removes the four JSON whitespace bytes
/// outside strings. Number spelling, escapes, duplicate keys, and key order are
/// preserved byte for byte. `None` means "not one JSON value", "already
/// compact", or "not safely confirmable", so callers keep the original string.
pub(super) fn compact_json_text(text: &str) -> Option<String> {
    if text.len() > MAX_COMPACT_JSON_TEXT_BYTES {
        return None;
    }
    // Cheap gate: only bytes that can start a JSON value get parsed. The full
    // check below still decides; `1 2` must not become `12`.
    let first = text.bytes().find(|byte| !is_json_whitespace(*byte))?;
    if !matches!(
        first,
        b'{' | b'[' | b'"' | b'-' | b't' | b'f' | b'n' | b'0'..=b'9'
    ) {
        return None;
    }
    let compact = strip_json_whitespace(text);
    if compact.len() == text.len() {
        return None;
    }
    // Validate the original, not the stripped text.
    serde_json::from_str::<Value>(text).ok()?;
    Some(compact)
}

fn is_json_whitespace(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r')
}

fn strip_json_whitespace(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_string = false;
    let mut escaped = false;
    for ch in text.chars() {
        if in_string {
            out.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
        } else if ch == '"' {
            in_string = true;
            out.push(ch);
        } else if !matches!(ch, ' ' | '\t' | '\n' | '\r') {
            out.push(ch);
        }
    }
    out
}

const TABLE_MIN_ROWS: usize = 3;

/// Guard against a row set whose union of keys explodes the header: a ragged
/// array with one unique key per row would otherwise render a useless table.
const TABLE_MAX_COLUMNS: usize = 16;

/// Rows and the trailing `… N more` marker added by [`degrade`], if any.
fn split_more(items: &[Value]) -> (&[Value], Option<usize>) {
    match items.split_last() {
        Some((last, rows)) => match is_more_marker(last) {
            Some(count) => (rows, Some(count)),
            None => (items, None),
        },
        None => (items, None),
    }
}

/// Columns of a table-shaped array, or `None` when it is not tabular.
///
/// Table shape: every row is an object, at least 3 rows (counting rows cut by
/// the budget), no row is empty, every value is primitive, and the union of
/// keys stays within `TABLE_MAX_COLUMNS`. Rows need not share the same key
/// set: the columns are the union in first-seen order (the first row's key
/// order first), and a row that omits a key renders it as an empty cell.
fn table_columns(items: &[Value]) -> Option<Vec<String>> {
    let (items, more) = split_more(items);
    if items.len() + more.unwrap_or(0) < TABLE_MIN_ROWS || items.is_empty() {
        return None;
    }
    let mut columns: Vec<String> = Vec::new();
    for item in items {
        let Value::Object(map) = item else {
            return None;
        };
        if map.is_empty() {
            return None;
        }
        for (key, value) in map {
            if matches!(value, Value::Array(_) | Value::Object(_)) {
                return None;
            }
            if !columns.iter().any(|column| column == key) {
                columns.push(key.clone());
                if columns.len() > TABLE_MAX_COLUMNS {
                    return None;
                }
            }
        }
    }
    Some(columns)
}

fn table_key(key: &str) -> String {
    let simple = !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '-' || c == '.' || c == '$');
    if simple { key.to_string() } else { quote(key) }
}

fn quote(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_else(|_| format!("\"{text}\""))
}

/// `key[N]{a,b,c}:` then one `v1,v2,v3` row per element. Columns come from
/// `table_columns`; a row missing a column renders that cell empty. Rows cut
/// by the budget end the block with `… N more rows`.
fn render_table(key: &str, items: &[Value], columns: &[String]) -> String {
    let (items, more) = split_more(items);
    let header: Vec<String> = columns.iter().map(|c| cell_text(c)).collect();
    let total = items.len() + more.unwrap_or(0);
    let mut out = format!("{key}[{total}]{{{}}}:", header.join(","));
    for item in items {
        let Value::Object(map) = item else { continue };
        let row: Vec<String> = columns
            .iter()
            .map(|c| cell(map.get(c.as_str()).unwrap_or(&Value::Null)))
            .collect();
        out.push('\n');
        out.push_str(&row.join(","));
    }
    if let Some(more) = more {
        out.push_str(&format!("\n… {more} more rows"));
    }
    out
}

fn cell(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(text) => cell_text(text),
        other => other.to_string(),
    }
}

/// Bare unless the text would be ambiguous: empty, separators/quotes/control
/// chars, edge whitespace, or something that reads as a number/bool/null.
fn cell_text(text: &str) -> String {
    let ambiguous = text.is_empty()
        || text.chars().any(|c| c == ',' || c == '"' || c.is_control())
        || text.trim() != text
        || matches!(text, "true" | "false" | "null")
        || text.parse::<f64>().is_ok();
    if ambiguous {
        quote(text)
    } else {
        text.to_string()
    }
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
        map.insert(tool.clone(), serde_json::json!({ "ok": ok, "fail": fail }));
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

fn render_header_block(report: &RunReport) -> String {
    let mut parts = vec![header_line(report)];
    if let Some(line) = &report.coerced_non_finite {
        parts.push(line.clone());
    }
    if report.failure_kind == Some(FailureKind::Exception)
        && let Some(stack) = short_stack(report.failure_stack.as_deref())
    {
        parts.push(stack);
    }
    // WP-210 H: dropped-undefined paths only matter when the run failed.
    if report.failure_kind.is_some()
        && let Some(line) = &report.dropped_undefined
    {
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
            if call_total(report) == 0 && report.entries.is_empty() =>
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
    let duration = format!("{}ms", report.duration_ms);
    let mut line = if let Some(error) = &report.tool_error {
        let detail = error
            .message
            .lines()
            .next()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(|line| prefix_utf8(line, HEAD_MESSAGE_CAP));
        let mut line = format!(
            "run_code failed: uncaught ToolError {} from call #{} {} {} · {duration}",
            error.code, error.seq, error.tool, error.summary
        );
        if let Some(detail) = detail {
            line.push_str(" · ");
            line.push_str(detail);
        }
        line
    } else if let Some(kind) = report.failure_kind {
        let label = if report.uncaught_type_error {
            "uncaught TypeError".to_string()
        } else {
            failure_label(kind, report)
        };
        let mut line = format!("run_code failed: {label} · {duration}");
        // WP-210 B: a runtime exception points at the user's line like a
        // syntax error does.
        if kind == FailureKind::Exception
            && let Some(location) = report.failure_location
        {
            line.push_str(&format!(
                " · line {}, column {}",
                location.line, location.column
            ));
            if let Some(snippet) = source_line_snippet(&report.source, location.line) {
                line.push_str(" · ");
                line.push_str(&snippet);
            }
        }
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
        let calls = call_total(report);
        format!("run_code ok · {calls} calls · {duration}")
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

/// Up to [`STACK_FRAMES`] frames from the user's program, with lines mapped
/// to the user's source (wrapper line removed). Runtime/libs frames are
/// dropped.
fn short_stack(stack: Option<&str>) -> Option<String> {
    let stack = stack?;
    let mut frames = Vec::new();
    for raw in stack.lines() {
        let frame = raw.trim();
        // WP-212: drop the wrapper `<eval>` frame by name (line≤1 already
        // covers the common `program.js:1` case; this catches any leftover).
        if frame.contains("at <eval>") {
            continue;
        }
        let Some(start) = frame.find("program.js:") else {
            continue;
        };
        let rest = &frame[start + "program.js:".len()..];
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        let Ok(line) = rest[..digits].parse::<u32>() else {
            continue;
        };
        if line <= 1 {
            continue;
        }
        let tail = &rest[digits..];
        let mapped = format!("{}line {}{}", &frame[..start], line - 1, tail);
        frames.push(format!("  {mapped}"));
        if frames.len() == STACK_FRAMES {
            break;
        }
    }
    (!frames.is_empty()).then(|| format!("stack:\n{}", frames.join("\n")))
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
    if report.failure_kind == Some(FailureKind::Timeout) && first.starts_with("run_code timed out")
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
        FailureKind::Cancelled
            if matches!(report.entries.last(), Some(EntryView::Value(_))) =>
        {
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


/// Cut `text` to `cap` and say what was omitted. Never returns an unmarked prefix.
/// This is the byte-cap side, so its marker names `byte cap` explicitly.
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
        let note = format!("… [truncated by byte cap: {omitted} bytes omitted of {total}]");
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn snap(seq: u64, tool: &str, summary: &str, state: &str, code: Option<&str>) -> LedgerSnap {
        LedgerSnap {
            seq,
            tool: tool.to_string(),
            summary: summary.to_string(),
            state: state.to_string(),
            error_code: code.map(str::to_string),
            session_read: false,
            brief: None,
        }
    }

    fn settled(seq: u64, tool: &str, summary: &str, state: &str, brief: &str) -> LedgerSnap {
        LedgerSnap {
            brief: Some(brief.to_string()),
            ..snap(seq, tool, summary, state, None)
        }
    }

    fn base_report() -> RunReport {
        RunReport {
            entries: Vec::new(),
            reminders: Vec::new(),
            failure_text: None,
            failure_kind: None,
            resource: None,
            timeout_ms: 120_000,
            duration_ms: 12,
            calls: Vec::new(),
            calls_omitted: 0,
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
            coerced_non_finite: None,
        }
    }

    fn text(level: &str, body: &str) -> EntryView {
        EntryView::Text {
            level: level.to_string(),
            text: body.to_string(),
        }
    }

    /// Put console entries before the value, the way the ledger does.
    fn set_logs(report: &mut RunReport, logs: Vec<(&str, String)>) {
        report
            .entries
            .retain(|entry| matches!(entry, EntryView::Value(_)));
        let mut views: Vec<EntryView> = logs
            .into_iter()
            .map(|(level, text)| EntryView::Text {
                level: level.to_string(),
                text,
            })
            .collect();
        views.append(&mut report.entries);
        report.entries = views;
    }

    /// Append the return value as the last entry.
    fn set_value(report: &mut RunReport, value: Value) {
        report
            .entries
            .retain(|entry| !matches!(entry, EntryView::Value(_)));
        report.entries.push(EntryView::Value(value));
    }

    fn tokens(text: &str) -> usize {
        crate::session::estimate::count_text_tokens(text)
    }


    /// A reminder lands after the entry its call produced.

    /// Every grep fact that lost its prose is still a field plus one line.
    #[test]
    fn grep_fields_and_their_reminder_lines() {
        let base = |extra: Value| {
            let mut value = serde_json::json!({
                "hits": [], "total_hits": 0, "at_limit_cap": false,
                "lines_truncated": false, "context_omitted": false, "output_capped": false,
            });
            let object = value.as_object_mut().unwrap();
            for (key, item) in extra.as_object().unwrap() {
                object.insert(key.clone(), item.clone());
            }
            value
        };
        let cases: Vec<(Value, Vec<&str>)> = vec![
            (
                base(serde_json::json!({"empty_reason": "no_files"})),
                vec!["nothing to search under this path"],
            ),
            (
                base(serde_json::json!({"empty_reason": "glob_no_files"})),
                vec!["no file matched the glob filter"],
            ),
            (
                base(serde_json::json!({"empty_reason": "no_match"})),
                vec!["the pattern was not found"],
            ),
            (
                base(serde_json::json!({"empty_reason": "file_unreadable"})),
                vec!["the file could not be read as text"],
            ),
            (
                base(serde_json::json!({"empty_reason": "binary"})),
                vec!["the file is binary; grep only searches text files"],
            ),
            (
                base(serde_json::json!({"empty_reason": "hidden_by_filters"})),
                vec!["the searched path is excluded by default filters"],
            ),
            (
                base(serde_json::json!({
                    "total_hits": 120, "at_limit_cap": true,
                    "hits": [{"path": "a.rs", "line": 1, "text": "needle"}],
                })),
                vec!["stopped at the hit limit; showing 1 of 120"],
            ),
            (
                base(serde_json::json!({
                    "total_hits": 9,
                    "hits": [{"path": "a.rs", "line": 1, "text": "needle"}],
                })),
                vec!["showing 1 of 9 matches"],
            ),
            (
                base(serde_json::json!({
                    "lines_truncated": true, "context_omitted": true, "output_capped": true,
                })),
                vec![
                    "long lines were shortened to fit the line cap",
                    "context lines were omitted to fit the budget",
                    "output was capped; narrow the pattern or path",
                ],
            ),
            (
                base(serde_json::json!({"clamped": [{"parameter": "before", "requested": 25, "applied": 10}]})),
                vec!["before 10 used instead of 25"],
            ),
        ];
        for (value, expected) in cases {
            let lines = reminders_for_call("file.grep", &value);
            assert_eq!(lines, expected, "{value}");
            for line in &lines {
                assert!(line.chars().count() <= REMINDER_MAX_CHARS, "{line}");
            }
        }
    }
    /// Every read fact that lost its prose is still a field plus one line.
    #[test]
    fn read_fields_and_their_reminder_lines() {
        let cases: Vec<(Value, Vec<&str>)> = vec![
            (
                serde_json::json!({
                    "path": "a.txt", "start_line": 1, "lines": ["a", "b"],
                    "total_lines": 2, "has_more": false, "skipped_in_context": 0,
                }),
                vec![],
            ),
            (
                serde_json::json!({
                    "path": "a.txt", "start_line": 1, "lines": ["a", "b"],
                    "total_lines": 9, "has_more": true, "skipped_in_context": 0,
                }),
                vec!["showing lines 1-2 of 9; continue with start_line 3"],
            ),
            (
                serde_json::json!({
                    "path": "a.txt", "start_line": 4, "lines": [],
                    "total_lines": 9, "has_more": true, "skipped_in_context": 2,
                }),
                vec![
                    "2 lines were left out because they are already in your context",
                    "showing lines 4-5 of 9; continue with start_line 6",
                ],
            ),
            (
                serde_json::json!({
                    "path": "a.txt", "start_line": 5, "lines": [],
                    "total_lines": 1, "has_more": false, "skipped_in_context": 0,
                }),
                vec!["start_line 5 is past the end; the file has 1 lines"],
            ),
            (
                serde_json::json!({
                    "path": "a.txt", "start_line": 1, "lines": ["x"],
                    "total_lines": 1, "has_more": false, "skipped_in_context": 0,
                    "lines_capped": [1, 7],
                }),
                vec!["lines 1, 7 shortened to fit the line cap"],
            ),
            (
                serde_json::json!({
                    "path": "a.txt", "start_line": 1, "lines": ["x"],
                    "total_lines": 1, "has_more": false, "skipped_in_context": 0,
                    "clamped": [{"parameter": "limit", "requested": 6000, "applied": 5000}],
                }),
                vec!["limit 5000 used instead of 6000"],
            ),
        ];
        for (value, expected) in cases {
            let lines = reminders_for_call("file.read", &value);
            assert_eq!(lines, expected, "{value}");
            for line in &lines {
                assert!(line.chars().count() <= REMINDER_MAX_CHARS, "{line}");
            }
        }
    }
    /// Every glob fact that has no prose any more still reaches the model as a
    /// field, plus exactly one reminder line. Table-driven on purpose: a field
    /// added without its line, or a line without its field, fails here.
    #[test]
    fn glob_fields_and_their_reminder_lines() {
        let cases: Vec<(Value, Vec<&str>)> = vec![
            (
                serde_json::json!({
                    "paths": ["a.rs", "b.rs"], "total": 2, "truncated": false,
                    "outside_workspace": false,
                }),
                vec![],
            ),
            (
                serde_json::json!({
                    "paths": ["a.rs"], "total": 200, "truncated": true,
                    "outside_workspace": false,
                }),
                vec!["showing first 1 of 200 paths; raise limit to see the rest"],
            ),
            (
                serde_json::json!({
                    "paths": [], "total": 0, "truncated": false,
                    "outside_workspace": false, "empty_reason": "hidden_by_filters",
                }),
                vec![
                    "the searched path is excluded by default filters; an empty page is not \"no matches\"",
                ],
            ),
            (
                serde_json::json!({
                    "paths": [], "total": 0, "truncated": false,
                    "outside_workspace": false, "empty_reason": "no_match",
                }),
                vec!["no path matched the pattern"],
            ),
            (
                serde_json::json!({
                    "paths": ["../x.rs"], "total": 1, "truncated": false,
                    "outside_workspace": true,
                }),
                vec!["paths outside the workspace are absolute"],
            ),
            (
                serde_json::json!({
                    "paths": ["a.rs"], "total": 1, "truncated": false,
                    "outside_workspace": false,
                    "clamped": [{"parameter": "limit", "requested": 6000, "applied": 5000}],
                }),
                vec!["limit 5000 used instead of 6000"],
            ),
        ];
        for (value, expected) in cases {
            let lines = reminders_for_call("file.glob", &value);
            assert_eq!(lines, expected, "{value}");
            for line in &lines {
                assert!(line.chars().count() <= REMINDER_MAX_CHARS, "{line}");
            }
            // The value itself keeps every field: nothing is hidden.
            assert!(value.is_object());
        }
    }

    /// LSP facts live in `state`. `partial` and `unknown` are different facts:
    /// one says the index is still building, the other says nobody reported it.
    #[test]
    fn lsp_fields_and_their_reminder_lines() {
        let miss = |subject: &str| {
            [
                format!("no {subject} reported; the index is ready, so this is a real miss"),
                format!(
                    "no {subject} reported; the index is still building, so this is not a confirmed miss"
                ),
                format!(
                    "no {subject} reported; the index state is unknown, so this is not a confirmed miss"
                ),
            ]
        };
        let locations = miss("locations");
        let diagnostics = miss("diagnostics");
        let symbols = miss("symbols");
        let hover = miss("hover information");
        let cases: Vec<(&str, Value, Vec<String>)> = vec![
            (
                "lsp.diagnostics",
                json!({"items": [], "state": "ok"}),
                vec![diagnostics[0].clone()],
            ),
            (
                "lsp.diagnostics",
                json!({"items": [], "state": "partial"}),
                vec![diagnostics[1].clone()],
            ),
            (
                "lsp.diagnostics",
                json!({"items": [], "state": "unknown"}),
                vec![diagnostics[2].clone()],
            ),
            (
                "lsp.diagnostics",
                json!({"items": [{"severity": "Error", "path": "a.rs", "line": 1,
                          "character": 1, "message": "x"}], "state": "partial"}),
                vec!["results may be incomplete; the index is still building".to_string()],
            ),
            (
                "lsp.diagnostics",
                json!({"items": [{"severity": "Error", "path": "a.rs", "line": 1,
                          "character": 1, "message": "x"}], "state": "unknown"}),
                vec!["results may be incomplete; the index state is unknown".to_string()],
            ),
            (
                "lsp.diagnostics",
                json!({"items": [{"severity": "Error", "path": "a.rs", "line": 1,
                          "character": 1, "message": "x"}], "state": "ok"}),
                Vec::new(),
            ),
            (
                "lsp.hover",
                json!({"markdown": "", "state": "ok"}),
                vec![hover[0].clone()],
            ),
            (
                "lsp.document_symbol",
                json!({"symbols": [], "state": "unknown"}),
                vec![symbols[2].clone()],
            ),
            (
                "lsp.workspace_symbol",
                json!({"symbols": [], "state": "partial"}),
                vec![symbols[1].clone()],
            ),
            (
                "lsp.find_references",
                json!({"locations": [], "state": "partial"}),
                vec![locations[1].clone()],
            ),
            (
                "lsp.go_to_implementation",
                json!({"locations": [], "state": "unknown"}),
                vec![locations[2].clone()],
            ),
            (
                "lsp.go_to_definition",
                json!({"locations": [{"path": "a.rs", "line": 1, "character": 1}], "state": "ok"}),
                Vec::new(),
            ),
        ];
        for (tool, value, expected) in cases {
            let lines = reminders_for_call(tool, &value);
            assert_eq!(lines, expected, "{tool} {value}");
            let mut report = base_report();
            report.reminders = lines
                .iter()
                .map(|text| CallReminder {
                    seq: 7,
                    tool: tool.to_string(),
                    text: text.clone(),
                })
                .collect();
            let out = render_model_view(&report);
            for text in &expected {
                let line = format!("↳ #7 {tool}: {text}");
                assert!(out.contains(&line), "{out}");
                assert!(line.chars().count() <= REMINDER_MAX_CHARS, "{line}");
            }
        }
    }
    /// Reminders describe calls, so they follow the stream in call order.
    #[test]
    fn call_reminders_follow_the_stream_in_call_order() {
        let mut report = base_report();
        report.entries = vec![text("log", "one")];
        report.reminders = vec![
            CallReminder {
                seq: 3,
                tool: "file.read".into(),
                text: "showing lines 1-2000 of 5000; continue with start_line 2001".into(),
            },
            CallReminder {
                seq: 4,
                tool: "file.grep".into(),
                text: "the pattern was not found".into(),
            },
        ];
        let out = render_model_view(&report);
        assert_eq!(
            out.lines().collect::<Vec<_>>(),
            vec![
                "run_code ok · 0 calls · 12ms",
                "",
                "[log] one",
                "",
                "↳ #3 file.read: showing lines 1-2000 of 5000; continue with start_line 2001",
                "↳ #4 file.grep: the pattern was not found",
            ],
            "{out}"
        );
    }

    /// One line, at most REMINDER_MAX_CHARS, and newlines are collapsed.
    #[test]
    fn a_reminder_is_one_short_line() {
        let mut report = base_report();
        report.entries = vec![text("log", "x")];
        let long = format!("{}\nsecond line", "y".repeat(400));
        report.reminders = vec![CallReminder {
            seq: 1,
            tool: "file.read".into(),
            text: long,
        }];
        let out = render_model_view(&report);
        let reminder = out
            .lines()
            .find(|line| line.starts_with("↳ #1 file.read: yyy"))
            .expect("reminder line");
        assert!(!reminder.contains("second line"), "{out}");
        assert_eq!(reminder.chars().count(), REMINDER_MAX_CHARS);
    }

    /// The budget covers reminders: they do not add a second allowance.
    #[test]
    fn reminders_are_charged_to_the_same_budget() {
        let mut report = base_report();
        report.entries = (0..200).map(|i| text("log", &format!("line {i}"))).collect();
        report.reminders = (0..200)
            .map(|i| CallReminder {
                seq: i as u64,
                tool: "file.grep".into(),
                text: format!("reminder {i} {}", "z".repeat(100)),
            })
            .collect();
        let out = render_model_view(&report);
        assert!(tokens(&out) <= RUN_CODE_VIEW_TOKENS, "{} tokens", tokens(&out));
    }

    /// A reminder is distinguishable from a bare returned string.
    #[test]
    fn a_reminder_is_marked_next_to_a_bare_value() {
        let mut report = base_report();
        report.entries = vec![
            EntryView::Value(serde_json::json!("showing lines 1-500 of 5000")),
            text("log", "done"),
        ];
        report.reminders = vec![CallReminder {
            seq: 2,
            tool: "file.read".into(),
            text: "showing lines 1-500 of 5000".into(),
        }];
        let out = render_model_view(&report);
        let lines: Vec<&str> = out.lines().collect();
        assert!(
            lines.iter().any(|line| *line == "showing lines 1-500 of 5000"),
            "the bare value keeps no mark: {out}"
        );
        assert!(
            lines.iter().any(|line| line.starts_with("↳ #2 file.read: ")),
            "the reminder carries the mark: {out}"
        );
    }

    #[test]
    fn one_budget_covers_the_whole_view() {
        assert_eq!(RUN_CODE_VIEW_TOKENS, 10_000);
        assert_eq!(RUN_CODE_VIEW_MAX_BYTES, 65_536);
    }

    #[test]
    fn no_legacy_sections_in_the_view() {
        let mut report = base_report();
        report.entries = vec![
            text("log", "one"),
            text("warn", "two"),
            EntryView::Value(serde_json::json!({ "ok": true, "n": 1 })),
        ];
        let out = render_model_view(&report);
        assert!(!out.contains("logs:"), "{out}");
        assert!(!out.contains("result:"), "{out}");
        assert!(!out.contains("program: ok"), "{out}");
        assert!(!out.contains("tool calls:"), "{out}");
    }

    #[test]
    fn success_stream_keeps_console_order_then_the_value() {
        let mut report = base_report();
        report.call_total = Some(3);
        report.entries = vec![
            text("log", "first"),
            text("warn", "second"),
            EntryView::Value(serde_json::json!({ "done": true, "ok": true })),
        ];
        let out = render_model_view(&report);
        assert_eq!(
            out,
            "run_code ok · 3 calls · 12ms\n\n[log] first\n[warn] second\n{\"done\":true,\"ok\":true}",
            "{out}"
        );
    }

    #[test]
    fn failure_view_is_header_manifest_then_stream() {
        let mut report = base_report();
        report.failure_kind = Some(FailureKind::Exception);
        report.failure_text = Some("boom".into());
        report.failure_location = Some(SourceLocation { line: 3, column: 5 });
        report.source = "a\nb\nboom();".into();
        report.calls = vec![settled(1, "write", "path=a.txt", "ok", "a.txt")];
        report.call_total = Some(1);
        report.entries = vec![text("log", "before")];
        let out = render_model_view(&report);
        // Snapshot: header, then the settled-call brief, then the same stream.
        assert_eq!(
            out,
            "run_code failed: exception · 12ms · line 3, column 5 · boom(); · boom\n\n#1 write path=a.txt → ok: a.txt\n\n[log] before",
            "{out}"
        );
    }

    #[test]
    fn manifest_is_charged_to_the_same_budget() {
        let mut report = base_report();
        report.failure_kind = Some(FailureKind::Exception);
        report.calls = (0..400)
            .map(|i| settled(i + 1, "write", "path=f.txt", "ok", "f.txt"))
            .collect();
        report.call_total = Some(400);
        report.entries = (0..200).map(|i| text("log", &format!("line {i}"))).collect();
        let out = render_model_view(&report);
        assert!(
            tokens(&out) <= RUN_CODE_VIEW_TOKENS + 64,
            "manifest and stream share one budget: {} tokens",
            tokens(&out)
        );
    }

    #[test]
    fn over_budget_stream_keeps_order_and_marks_the_cut() {
        let mut report = base_report();
        let body = "word ".repeat(12_000);
        report.entries = vec![
            text("log", &body),
            text("log", "MIDDLE_SENTINEL"),
            EntryView::Value(serde_json::json!({ "tail": "TAIL_SENTINEL" })),
        ];
        let out = render_model_view(&report);
        assert!(tokens(&out) <= RUN_CODE_VIEW_TOKENS, "{} tokens", tokens(&out));
        assert!(
            out.contains("truncated by token budget")
                || out.contains("entries omitted (token budget)")
                || out.contains("truncated by byte cap"),
            "the cut names the budget: {}",
            &out[out.len().saturating_sub(300)..]
        );
        // The first entry keeps its head, the stream keeps its order.
        assert!(out.contains("[log] word word"), "{out}");
        let first = out.find("[log] word word").unwrap();
        let middle = out.find("MIDDLE_SENTINEL").expect("later entries survive");
        assert!(first < middle, "order is preserved: {out}");
    }

    #[test]
    fn the_value_is_not_reserved_a_separate_share() {
        let mut report = base_report();
        report.entries = (0..40)
            .map(|i| text("log", &format!("line {i} {}", "x".repeat(400))))
            .chain(std::iter::once(EntryView::Value(serde_json::json!({
                "payload": "v".repeat(200_000)
            }))))
            .collect();
        let out = render_model_view(&report);
        assert!(tokens(&out) <= RUN_CODE_VIEW_TOKENS, "{} tokens", tokens(&out));
        // The value competes with the log lines: it is cut like any entry.
        assert!(
            out.contains("truncated by token budget")
                || out.contains("… [value omitted")
                || out.contains("entries omitted (token budget)"),
            "{}",
            &out[out.len().saturating_sub(400)..]
        );
    }


    /// Over budget, nothing fits: the stream degrades to one marker line.
    #[test]
    fn over_budget_stream_snapshot() {
        let entries = vec![
            text("log", "alpha alpha alpha alpha"),
            text("log", "beta beta beta beta"),
        ];
        assert_eq!(
            render_stream(&entries, 12, false),
            "… 2 entries omitted (token budget)"
        );
        // Enough room: the same stream, whole, in order.
        assert_eq!(
            render_stream(&entries, 40, false),
            "[log] alpha alpha alpha alpha\n[log] beta beta beta beta"
        );
    }

    #[test]
    fn many_entries_fall_back_to_a_head_prefix() {
        let mut report = base_report();
        report.entries = (0..2_000)
            .map(|i| text("log", &format!("entry {i} {}", "z".repeat(200))))
            .collect();
        let out = render_model_view(&report);
        assert!(tokens(&out) <= RUN_CODE_VIEW_TOKENS, "{} tokens", tokens(&out));
        assert!(out.contains("entries omitted (token budget)"), "{out}");
    }

    #[test]
    fn byte_fallback_keeps_the_tail_and_names_the_byte_cap() {
        let mut body = String::from("HEAD_SENTINEL");
        body.push_str(&"a".repeat(RUN_CODE_VIEW_MAX_BYTES));
        body.push_str("TAIL_SENTINEL");
        let cut = byte_cut(&body, RUN_CODE_VIEW_MAX_BYTES);
        assert!(cut.len() <= RUN_CODE_VIEW_MAX_BYTES + 120, "{}", cut.len());
        assert!(cut.contains("HEAD_SENTINEL"), "head kept");
        assert!(cut.contains("TAIL_SENTINEL"), "tail kept");
        assert!(cut.contains("truncated by byte cap"), "{cut}");
        // A huge payload never escapes the cap through the model view.
        let mut report = base_report();
        set_logs(&mut report, vec![("log", "中".repeat(60_000))]);
        let out = render_model_view(&report);
        assert!(out.len() <= RUN_CODE_VIEW_MAX_BYTES + 120, "{}", out.len());
    }

    #[test]
    fn typical_stream_fits_under_the_byte_cap() {
        let mut report = base_report();
        set_logs(
            &mut report,
            (0..40).map(|i| ("log", format!("line {i} of a normal run"))).collect(),
        );
        set_value(&mut report, serde_json::json!({ "ok": true, "paths": ["a", "b"] }));
        let out = render_model_view(&report);
        assert!(out.len() < RUN_CODE_VIEW_MAX_BYTES / 4, "{} bytes", out.len());
    }

    #[test]
    fn oversized_entry_keeps_head_and_tail() {
        let mut report = base_report();
        let body = format!("HEAD{}\n{}TAIL", "m".repeat(60_000), "n".repeat(60_000));
        report.entries = vec![text("log", &body)];
        let out = render_model_view(&report);
        assert!(tokens(&out) <= RUN_CODE_VIEW_TOKENS, "{} tokens", tokens(&out));
        assert!(out.contains("HEAD"), "{out}");
        assert!(out.contains("TAIL"), "{out}");
    }


    #[test]
    fn dropped_undefined_and_non_finite_lines_sit_in_the_header() {
        let mut report = base_report();
        report.failure_kind = Some(FailureKind::Exception);
        report.dropped_undefined = Some("dropped undefined at a".into());
        let head = render_model_view(&report);
        assert!(head.contains("dropped undefined at a"), "{head}");
        let mut clean = base_report();
        clean.coerced_non_finite = Some("coerced non-finite number at n to null".into());
        let head = render_model_view(&clean);
        assert!(
            head.contains("coerced non-finite number at n to null"),
            "{head}"
        );
    }

    #[test]
    fn cjk_value_is_cut_by_tokens_not_bytes() {
        let body = "中".repeat(40_000);
        let out = budget_value(&serde_json::json!(body), 400);
        assert!(out.contains("truncated by token budget"), "{out}");
        assert!(out.chars().count() < 40_000, "{out}");
    }
    #[test]
    fn header_counts_calls_and_names_the_failure() {
        let mut report = base_report();
        report.call_total = Some(4);
        report.duration_ms = 1234;
        assert_eq!(header_line(&report), "run_code ok · 4 calls · 1234ms");

        report.failure_kind = Some(FailureKind::Timeout);
        report.failure_text = Some("run_code timed out after 120000ms".into());
        assert_eq!(
            header_line(&report),
            "run_code failed: timeout after 120000ms · 1234ms"
        );
    }

    #[test]
    fn uncaught_tool_error_header_keeps_the_code_and_the_call() {
        let mut report = base_report();
        report.tool_error = Some(UncaughtToolError {
            seq: 2,
            tool: "write".into(),
            code: "invalid_args".into(),
            message: "nope".into(),
            summary: "path=a.txt".into(),
        });
        report.call_total = Some(2);
        let line = header_line(&report);
        assert!(line.contains("uncaught ToolError invalid_args"), "{line}");
        assert!(line.contains("from call #2 write"), "{line}");
        assert!(!line.contains("tool calls:"), "{line}");
    }

    #[test]
    fn uniform_object_array_renders_as_a_table() {
        let value = serde_json::json!([
            { "a": 1, "b": "two" },
            { "a": 3, "b": "four" },
            { "a": 5, "b": "six" }
        ]);
        let body = render_result_body(&value);
        assert!(body.starts_with("[3]{a,b}:"), "{body}");
        assert!(body.contains("1,two"), "{body}");
    }

    #[test]
    fn non_tabular_arrays_fall_back_to_compact_json() {
        let value = serde_json::json!([{ "a": [1, 2] }, { "a": [3] }]);
        let body = render_result_body(&value);
        assert_eq!(body, r#"[{"a":[1,2]},{"a":[3]}]"#);
    }

    #[test]
    fn strings_stay_verbatim_and_json_text_is_compacted() {
        let plain = serde_json::json!("hello world");
        assert_eq!(render_result_body(&plain), "hello world");
        let json_text = serde_json::json!("{ \"a\": 1, \"b\": [1, 2] }");
        assert_eq!(render_result_body(&json_text), r#"{"a":1,"b":[1,2]}"#);
    }

    #[test]
    fn human_data_stays_structured_for_the_pretty_frontend() {
        let mut report = base_report();
        report.calls = vec![settled(1, "write", "path=a.txt", "ok", "a.txt")];
        report.call_total = Some(1);
        report.by_tool.insert("write".into(), (1, 0));
        let data = human_data(&report).expect("data");
        assert_eq!(data["ok"], true);
        assert_eq!(data["succeeded"], 1);
        assert_eq!(data["failed"], 0);
        assert_eq!(data["by_tool"]["write"], serde_json::json!({ "ok": 1, "fail": 0 }));
    }
    #[test]
    fn json_text_is_compacted_without_rewriting_tokens() {
        let pretty = r#"{
  "a": 1.00,
  "a": 2,
  "e": 1e2,
  "u": "\u0041",
  "s": " x  y ",
  "q": "a\"b",
  "b": "c\\d",
  "中文": "值"
}"#;
        assert_eq!(
            compact_json_text(pretty).as_deref(),
            Some(r#"{"a":1.00,"a":2,"e":1e2,"u":"\u0041","s":" x  y ","q":"a\"b","b":"c\\d","中文":"值"}"#)
        );
        assert_eq!(
            compact_json_text("  {\"a\": 1}  ").as_deref(),
            Some(r#"{"a":1}"#)
        );
        assert_eq!(
            compact_json_text("{\r\n  \"a\": 1\r\n}").as_deref(),
            Some(r#"{"a":1}"#)
        );
        assert_eq!(
            compact_json_text("{\"s\": \"c\\\\\"}").as_deref(),
            Some(r#"{"s":"c\\"}"#)
        );
        assert_eq!(
            compact_json_text("{\"s\": \"c\\\"\"}").as_deref(),
            Some(r#"{"s":"c\""}"#)
        );
    }

    #[test]
    fn json_text_compaction_handles_top_level_scalars() {
        assert_eq!(compact_json_text("  42  ").as_deref(), Some("42"));
        assert_eq!(compact_json_text("  \"hi\"  ").as_deref(), Some("\"hi\""));
        assert_eq!(compact_json_text("  true  ").as_deref(), Some("true"));
        assert_eq!(compact_json_text("  null  ").as_deref(), Some("null"));
        assert_eq!(
            compact_json_text("  \"a b\"  ").as_deref(),
            Some("\"a b\"")
        );
        assert_eq!(
            compact_json_text("  \"a\\\"b\"  ").as_deref(),
            Some("\"a\\\"b\"")
        );
        assert_eq!(compact_json_text("  \"a\nb\"  "), None);
        assert_eq!(compact_json_text("42"), None);
    }

    #[test]
    fn json_text_compaction_preserves_the_parsed_value() {
        let values = [
            serde_json::json!({"a": [1, 2, {"b": "x y"}], "c": true, "d": null}),
            serde_json::json!(["x", {"y": [1.5, 2e10]}]),
            serde_json::json!({"s": "quote \" and \\ slash", "t": "line\nbreak"}),
        ];
        for value in values {
            let pretty = serde_json::to_string_pretty(&value).unwrap();
            let compact = compact_json_text(&pretty).expect("pretty JSON compacts");
            assert_eq!(serde_json::from_str::<Value>(&compact).unwrap(), value);
            assert_eq!(compact, serde_json::to_string(&value).unwrap());
        }
    }

    #[test]
    fn json_text_compaction_rejects_non_single_json() {
        for text in [
            "hello",
            "{\"a\":1,}",
            "{\"a\":1} // x",
            "{\"a\":1}\n{\"b\":2}",
            "1 2",
            "{\n  \"a\": 1\n} trailing",
        ] {
            assert_eq!(compact_json_text(text), None, "{text}");
        }
        assert_eq!(compact_json_text(r#"{"a":1}"#), None);
    }

    #[test]
    fn pretty_json_string_result_is_compacted() {
        let text = ok_with(Value::String(
            "{\n  \"a\": 1,\n  \"b\": [true, null]\n}".into(),
        ));
        assert_eq!(result_of(&text), r#"{"a":1,"b":[true,null]}"#);
    }

    #[test]
    fn pretty_json_log_lines_are_compacted() {
        let mut report = base_report();
        set_logs(
            &mut report,
            vec![
                ("log", "{\n  \"a\": 1\n}".to_string()),
                ("log", "plain text".to_string()),
            ],
        );
        let text = render_model_view(&report);
        assert!(text.contains("[log] {\"a\":1}"), "{text}");
        assert!(text.contains("[log] plain text"), "{text}");
        assert!(!text.contains("[log] {\n"), "{text}");
    }

    #[test]
    fn large_string_result_keeps_head_and_tail_under_the_token_budget() {
        let mut report = base_report();
        let body = format!(
            "{}RESULT_END_SENTINEL",
            (0..20_000)
                .map(|i| format!("line {i} alpha beta\n"))
                .collect::<String>()
        );
        set_value(&mut report, Value::String(body));
        let text = render_model_view(&report);
        assert!(text.starts_with("run_code ok"));
        assert!(text.len() <= RUN_CODE_VIEW_MAX_BYTES, "{}", text.len());
        let result = result_of(&text);
        assert!(
            result.contains("truncated by token budget"),
            "{result}"
        );
        assert!(result.contains("line 0 alpha"), "{result}");
        assert!(result.contains("RESULT_END_SENTINEL"), "{result}");
        assert!(result.contains("tok omitted from the middle"), "{result}");
        assert!(!result.contains("byte cap"), "{result}");
        assert!(!text.starts_with("[blob:"));
    }

    #[test]
    fn over_budget_object_degrades_structurally_and_stays_valid_json() {
        let items: Vec<Value> = (0..4_000)
            .map(|i| serde_json::json!({"id": i, "tags": ["x", "y"], "note": "word ".repeat(10)}))
            .collect();
        let text = ok_with(serde_json::json!({"total": 4000, "items": items, "done": true}));
        let result = result_of(&text);
        let (notice, body) = result.split_once('\n').unwrap();
        assert!(notice.contains("exceeds 10.0k; truncated"), "{notice}");
        let parsed: Value = serde_json::from_str(body).expect("still valid JSON");
        assert_eq!(parsed["total"], 4000);
        assert_eq!(parsed["done"], true);
        let kept = parsed["items"].as_array().unwrap();
        let marker = kept.last().unwrap().as_str().unwrap();
        assert_eq!(marker, format!("… {} more", 4000 - (kept.len() - 1)));
        assert!(crate::session::estimate::count_text_tokens(body) <= RUN_CODE_VIEW_TOKENS);
        // Key order of the agent survives degradation.
        assert!(
            body.starts_with("{\"total\":4000,\"items\":["),
            "{}",
            &body[..40]
        );
    }

    #[test]
    fn long_strings_are_cut_before_containers_collapse_to_key_counts() {
        // Shape from a real run: four file reads returned as {path,n,text}.
        // At ~11k tokens the result must keep useful prefixes, not become
        // {"catalog":"{…3 keys}",...}, which drops the whole payload.
        // Sized so the degraded object stays under the byte cap: this test is
        // about strings being cut before containers collapse, not about the cap.
        let value = serde_json::json!({
            "catalog": {"path": "src/apps/catalog.rs", "n": 660, "text": "x".repeat(8_000)},
            "app_panel": {"path": "web/src/lib/appPanel.ts", "n": 420, "text": "x".repeat(8_000)},
            "app_tab": {"path": "web/src/dockview/tabs/AppTab.tsx", "n": 310, "text": "x".repeat(8_000)},
            "app_view": {"path": "web/src/dockview/panels/AppViewPanel.tsx", "n": 280, "text": "x".repeat(8_000)},
        });
        let text = ok_with(value);
        let result = result_of(&text);
        let body = match result.split_once('\n') {
            Some((notice, rest)) if notice.contains("exceeds") => rest,
            _ => result,
        };
        let parsed: Value = serde_json::from_str(body).expect("still valid JSON");
        for (key, path) in [
            ("catalog", "src/apps/catalog.rs"),
            ("app_panel", "web/src/lib/appPanel.ts"),
            ("app_tab", "web/src/dockview/tabs/AppTab.tsx"),
            ("app_view", "web/src/dockview/panels/AppViewPanel.tsx"),
        ] {
            assert_eq!(parsed[key]["path"].as_str(), Some(path), "{body}");
            let kept = parsed[key]["text"].as_str().expect("text kept");
            assert!(kept.starts_with('x'), "{kept}");
            assert!(kept.len() > 1_000, "{}", kept.len());
        }
        assert!(!body.contains("{…"), "{body}");
        assert!(crate::session::estimate::count_text_tokens(body) <= RUN_CODE_VIEW_TOKENS);
    }

    #[test]
    fn output_limit_mark_survives_an_overlong_last_line() {
        let mut report = base_report();
        report.failure_kind = Some(FailureKind::OutputLimit);
        report.failure_text = Some("output exceeded 1048576 bytes".into());
        set_logs(&mut report, vec![("info", "中".repeat(40_000))]);
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
    fn short_stack_drops_eval_frames_by_name() {
        let mut report = base_report();
        report.failure_kind = Some(FailureKind::Exception);
        report.failure_text = Some("x".into());
        report.failure_location = Some(SourceLocation { line: 2, column: 1 });
        report.source = "return 1;\nthrow new Error('x');".into();
        report.failure_stack = Some(
            "    at <eval> (program.js:3:1)\n    at boom (program.js:3:5)\n    at <eval> (program.js:9:9)".into(),
        );
        let text = render_model_view(&report);
        assert!(text.contains("stack:\n  at boom (line 2:5)"), "{text}");
        assert!(!text.contains("<eval>"), "{text}");
    }

    #[test]
    fn ragged_rows_render_as_a_padded_table() {
        let text = ok_with(serde_json::json!([
            {"a": 1, "b": 2},
            {"a": 3},
            {"b": 5, "a": 4},
        ]));

        // Columns are the union in first-seen order; an omitted key is empty.
        assert_eq!(result_of(&text), "[3]{a,b}:\n1,2\n3,\n4,5");
    }

    #[test]
    fn later_row_new_key_appends_a_column() {
        let text = ok_with(serde_json::json!([
            {"a": 1},
            {"a": 2},
            {"a": 3, "b": 4},
        ]));
        assert_eq!(result_of(&text), "[3]{a,b}:\n1,\n2,\n3,4");
    }

    #[test]
    fn too_many_columns_fall_back_to_compact_json() {
        let mut rows = Vec::new();
        for row in 0..3 {
            let mut map = serde_json::Map::new();
            for key in 0..17 {
                map.insert(format!("k{key}"), serde_json::json!(row));
            }
            rows.push(Value::Object(map));
        }
        let value = Value::Array(rows);
        let text = ok_with(value.clone());
        assert_eq!(result_of(&text), serde_json::to_string(&value).unwrap());
    }

    #[test]
    fn table_cells_quote_ambiguous_strings() {
        let text = ok_with(serde_json::json!([
            {"v": "a,b"}, {"v": "say \"hi\""}, {"v": "x\ny"},
            {"v": " pad"}, {"v": ""}, {"v": "42"}, {"v": "true"}, {"v": "null"}, {"v": "中文 ok"},
        ]));
        let rows: Vec<&str> = result_of(&text).lines().collect();
        assert_eq!(rows[0], "[9]{v}:");
        assert_eq!(
            &rows[1..],
            &[
                r#""a,b""#,
                r#""say \"hi\"""#,
                r#""x\ny""#,
                r#"" pad""#,
                r#""""#,
                r#""42""#,
                r#""true""#,
                r#""null""#,
                "中文 ok"
            ]
        );
    }

    #[test]
    fn table_nested_in_an_object_renders_as_a_keyed_block() {
        let text = ok_with(serde_json::json!({
            "total": 3,
            "hits": [{"p": "a", "n": 1}, {"p": "b", "n": 2}, {"p": "c", "n": 3}],
            "note": "x"
        }));
        let result = result_of(&text);
        assert_eq!(
            result,
            "{\"total\":3,\"note\":\"x\"}\nhits[3]{p,n}:\na,1\nb,2\nc,3"
        );
    }

    #[test]
    fn table_is_truncated_on_whole_rows() {
        let rows: Vec<Value> = (0..5_000)
            .map(|i| serde_json::json!({"path": format!("src/mod_{i}/file.rs"), "line": i}))
            .collect();
        let text = ok_with(Value::Array(rows));
        let result = result_of(&text);
        let body: Vec<&str> = result.lines().collect();
        assert!(body[1].starts_with("[5000]{path,line}:"), "{}", body[1]);
        // Whole rows only, then one `… N more rows` line (WP-210 C).
        let marker = body[body.len() - 1];
        let shown = body.len() - 3;
        assert_eq!(marker, format!("… {} more rows", 5000 - shown), "{marker}");
        let last_row = body[body.len() - 2];
        let (path, line) = last_row.rsplit_once(',').unwrap();
        assert_eq!(path, format!("src/mod_{line}/file.rs"));
    }

    #[test]
    fn other_values_are_compact_json_and_strings_verbatim() {
        let value = serde_json::json!({"a": {"b": [1, 2]}, "c": "d"});
        let text = ok_with(value.clone());
        assert_eq!(result_of(&text), serde_json::to_string(&value).unwrap());
        assert!(!result_of(&text).contains('\n'));
        let text = ok_with(Value::String("plain\n  text".into()));
        assert_eq!(result_of(&text), "plain\n  text");
    }

    #[test]
    fn failure_header_counts_and_marks_unsettled() {
        let mut report = base_report();
        report.failure_kind = Some(FailureKind::Timeout);
        report.failure_text = Some("run_code timed out after 120000ms".into());
        report.duration_ms = 120_000;
        report.calls = vec![
            snap(1, "read", "path=a", "ok", None),
            snap(2, "write", "path=b", "unsettled", None),
        ];
        report.calls_omitted = 2;
        report.call_total = Some(4);
        report.unsettled_total = Some(1);
        let text = render_model_view(&report);
        assert!(
            text.starts_with("run_code failed: timeout after 120000ms · 120000ms"),
            "{text}"
        );
        // Unsettled keeps its own head line; it is not folded into the stream.
        assert!(text.contains("1 calls were still running"), "{text}");
        assert!(!text.contains("tool calls:"), "{text}");
    }

    /// Everything after the header: the stream (these tests render success
    /// views, where the header is followed directly by the stream).
    fn result_of(text: &str) -> &str {
        text.split_once("\n\n").map(|(_, rest)| rest).unwrap_or("")
    }

    fn ok_with(value: Value) -> String {
        let mut report = base_report();
        set_value(&mut report, value);
        render_model_view(&report)
    }


}

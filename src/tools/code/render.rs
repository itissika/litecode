//! Model-facing text for one `run_code` execution.
//!
//! Default agent view: head, optional logs, optional partial results (failed
//! runs only), optional result. Result: uniformly shaped object arrays render
//! as `[N]{a,b}:` tables (columns in first-seen key order, a key a row omits
//! renders as an empty cell), everything else as compact JSON (strings
//! verbatim). Partial calls and the final result share
//! [`RESULT_TOKEN_BUDGET`] cl100k tokens; structured values degrade
//! structurally, while over-budget strings keep both a head and a tail
//! (WP-210 C). Logs keep their head within [`SUCCESS_LOGS_TOKEN_BUDGET`] on
//! success and their tail within [`FAILURE_LOGS_TOKEN_BUDGET`] on failure.
//! [`RUN_CODE_VIEW_MAX_BYTES`] remains the hard byte fallback shared by logs
//! and result. Cuts land on UTF-8 boundaries, and every omission marker says
//! whether the token budget or the byte cap caused it. This module does not
//! emit a JSON copy of the run.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::ptc::{FailureKind, ResourceKind, SourceLocation};
use crate::session::transcript_file::RUN_CODE_SESSION_NOTE;
use crate::types::utf8::{prefix_utf8, suffix_utf8};

/// Hard cap for the model-facing `run_code` view, including omission notes.
pub const RUN_CODE_VIEW_MAX_BYTES: usize = 65_536;

const RESERVE_BYTES: usize = 512;
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
    let fixed = vec![render_header_block(report)];
    let succeeded = report.failure_kind.is_none();
    let logs = if succeeded {
        render_success_logs(&report.logs, SUCCESS_LOGS_TOKEN_BUDGET)
    } else {
        render_failure_logs(
            &report.logs,
            FAILURE_LOGS_TOKEN_BUDGET,
            report.output_limited,
        )
    };

    // Partial calls and the final return value are two windows onto one
    // result budget. Keep a small reserve for the final result when both are
    // present, then spend exactly what the partial block did not use.
    let partial_budget = if report.value.is_some() {
        RESULT_TOKEN_BUDGET.saturating_sub(FINAL_RESULT_RESERVE_TOKENS)
    } else {
        RESULT_TOKEN_BUDGET
    };
    let partial = render_partial_results(report, partial_budget);
    let partial_tokens = partial.as_deref().map(count_tokens).unwrap_or(0);
    let mut final_budget = RESULT_TOKEN_BUDGET.saturating_sub(partial_tokens);
    let mut result_body = report
        .value
        .as_ref()
        .map(|value| budget_value(value, final_budget))
        .filter(|body| !body.is_empty());
    // BPE counts are not perfectly additive at section boundaries. Measure the
    // rendered partial+final result area and shrink the final value until the
    // whole area fits the single shared result budget.
    loop {
        let candidate = result_body.as_ref().map(|body| format!("result:\n{body}"));
        let combined = match (&partial, &candidate) {
            (Some(partial), Some(result)) => format!("{partial}\n\n{result}"),
            (Some(partial), None) => partial.clone(),
            (None, Some(result)) => result.clone(),
            (None, None) => String::new(),
        };
        let used = count_tokens(&combined);
        if used <= RESULT_TOKEN_BUDGET || final_budget == 0 {
            break;
        }
        final_budget = final_budget.saturating_sub((used - RESULT_TOKEN_BUDGET).max(1));
        result_body = report
            .value
            .as_ref()
            .map(|value| budget_value(value, final_budget))
            .filter(|body| !body.is_empty());
    }
    let result = result_body.as_ref().map(|body| format!("result:\n{body}"));

    let mut text = assemble(&fixed, &logs, &partial, &result);
    if text.len() <= RUN_CODE_VIEW_MAX_BYTES {
        return text;
    }

    // The token budgets normally fit in 64KB, but byte-heavy content can
    // still trip the byte cap. Logs and result sections keep distinct token
    // budgets while sharing this final byte ceiling.
    text = byte_fallback(&fixed, logs, partial, result_body.as_deref());
    text
}

/// Shared cl100k budget for partial results plus the final returned result.
/// Omission notices count against this cap.
pub const RESULT_TOKEN_BUDGET: usize = 10_000;
/// Token budget for logs on a failed run (the tail is kept).
pub const FAILURE_LOGS_TOKEN_BUDGET: usize = 3_000;
/// Token budget for logs on a successful run (the head is kept, WP-210 D/Q5).
pub const SUCCESS_LOGS_TOKEN_BUDGET: usize = 3_000;
/// Keep enough shared result budget for the final value when partial calls are
/// also present. The partial block gets the rest.
const FINAL_RESULT_RESERVE_TOKENS: usize = 512;
/// Program frames shown under a runtime exception (WP-210 B/Q3).
const STACK_FRAMES: usize = 5;
/// Maximum bytes the log section keeps before the final byte cap fills the
/// result side.
const LOGS_FALLBACK_CAP: usize = 16 * 1024;

fn count_tokens(text: &str) -> usize {
    crate::session::estimate::count_text_tokens(text)
}

/// Successful run: keep whole leading log lines within `budget` tokens, then
/// one `... N lines omitted (token budget)` line for the rest.
fn render_success_logs(logs: &[(String, String)], budget: usize) -> Option<String> {
    if logs.is_empty() || budget == 0 {
        return None;
    }
    let lines: Vec<String> = logs
        .iter()
        .map(|(level, message)| format!("[{level}] {message}"))
        .collect();
    let build = |count: usize, token_limited: bool| {
        let mut body = String::from("logs:");
        for line in &lines[..count] {
            body.push('\n');
            body.push_str(line);
        }
        let omitted = lines.len() - count;
        if omitted > 0 {
            body.push_str(&format!("\n… {omitted} lines omitted"));
            if token_limited {
                body.push_str(" (token budget)");
            }
        }
        body
    };
    let full = build(lines.len(), false);
    if full.len() <= budget || count_tokens(&full) <= budget {
        return Some(full);
    }
    let (mut lo, mut hi) = (0usize, lines.len());
    while lo < hi {
        let mid = (lo + hi + 1) / 2;
        if count_tokens(&build(mid, true)) <= budget {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    let rendered = build(lo, true);
    fits(&rendered, budget).then_some(rendered)
}

/// Failed run that had started: one line per settled nested call so the
/// work already done is not lost (WP-210 A/Q1). The partial block spends from
/// the same result budget as the final value; whole lines are dropped from the
/// end and the omission marker names the token-budget side.
fn render_partial_results(report: &RunReport, budget: usize) -> Option<String> {
    if report.failure_kind.is_none() || did_not_start(report) || budget == 0 {
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
    let lines: Vec<String> = settled
        .iter()
        .map(|call| {
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
            format!("#{} {}{summary} → {status}: {brief}", call.seq, call.tool)
        })
        .collect();
    let hidden = call_total(report).saturating_sub(report.calls.len());
    let build = |count: usize, token_limited: bool| {
        let mut out = String::from("completed tool calls before the failure:");
        for line in &lines[..count] {
            out.push('\n');
            out.push_str(line);
        }
        let more = lines.len() - count + hidden;
        if more > 0 {
            out.push_str(&format!("\n… {more} more"));
            if token_limited {
                out.push_str(" (token budget)");
            }
        }
        out
    };
    let full = build(lines.len(), false);
    if full.len() <= budget || count_tokens(&full) <= budget {
        return Some(full);
    }
    let (mut lo, mut hi) = (0usize, lines.len());
    while lo < hi {
        let mid = (lo + hi + 1) / 2;
        if count_tokens(&build(mid, true)) <= budget {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    let rendered = build(lo, true);
    fits(&rendered, budget).then_some(rendered)
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
            format!("{notice}\n{head}{marker}")
        } else {
            format!("{notice}\n{head}{marker}{tail}")
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
    let compact = format!("{notice}\n… [truncated by token budget: 0 of {total} tok kept]");
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

/// Failed run: keep the tail of the logs within `budget` tokens (errors sit at
/// the end) and say how many earlier lines were dropped.
fn render_failure_logs(
    logs: &[(String, String)],
    budget: usize,
    output_limited: bool,
) -> Option<String> {
    if logs.is_empty() || budget == 0 {
        return None;
    }
    let mut lines: Vec<String> = logs
        .iter()
        .map(|(level, message)| format!("[{level}] {message}"))
        .collect();
    if output_limited {
        if let Some(last) = lines.last_mut() {
            last.push_str(OUTPUT_LIMIT_MARK);
        }
    }
    let build = |start: usize, lines: &[String]| {
        let mut body = String::from("logs:");
        if start > 0 {
            body.push_str(&format!(
                "\n… [{start} earlier lines omitted] (token budget)"
            ));
        }
        for line in &lines[start..] {
            body.push('\n');
            body.push_str(line);
        }
        body
    };
    let full = build(0, &lines);
    if full.len() <= budget || count_tokens(&full) <= budget {
        return Some(full);
    }
    // Smallest start whose rendered tail fits.
    let (mut lo, mut hi) = (0usize, lines.len());
    while lo < hi {
        let mid = (lo + hi) / 2;
        if count_tokens(&build(mid, &lines)) <= budget {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    if lo < lines.len() {
        return Some(build(lo, &lines));
    }
    // Even the last line alone is too long: keep its tail (the limit mark stays).
    let start = lines.len() - 1;
    let overhead = count_tokens(&format!(
        "logs:\n… [{start} earlier lines omitted] (token budget)\n… [truncated by token budget]"
    )) + 8;
    let last = lines.pop().unwrap_or_default();
    let tail = token_suffix(&last, budget.saturating_sub(overhead)).to_string();
    lines.push(format!("… [truncated by token budget] {tail}"));
    let rendered = build(start, &lines);
    fits(&rendered, budget).then_some(rendered)
}

fn render_result_body(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
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

fn assemble(
    fixed: &[String],
    logs: &Option<String>,
    partial: &Option<String>,
    result: &Option<String>,
) -> String {
    let mut sections = fixed.to_vec();
    if let Some(logs) = logs {
        sections.push(logs.clone());
    }
    if let Some(partial) = partial {
        sections.push(partial.clone());
    }
    if let Some(result) = result {
        sections.push(result.clone());
    }
    sections.join("\n\n")
}

/// Final 64KB guard. Logs keep a bounded byte share; the result side (partial
/// calls plus the final value) fills the rest. Every byte-side cut says so in
/// its omission marker, so it cannot be confused with the token budgets.
fn byte_fallback(
    fixed: &[String],
    mut logs: Option<String>,
    mut partial: Option<String>,
    result_body: Option<&str>,
) -> String {
    let cap = RUN_CODE_VIEW_MAX_BYTES;
    let fixed_only = assemble(fixed, &None, &None, &None);
    let has_result_side = partial.is_some() || result_body.is_some();

    if let Some(section) = logs.take() {
        let mut logs_cap = cap
            .saturating_sub(fixed_only.len())
            .saturating_sub(2)
            .saturating_sub(RESERVE_BYTES);
        if has_result_side {
            logs_cap = logs_cap.saturating_sub(2);
        }
        logs_cap = logs_cap.min(LOGS_FALLBACK_CAP);
        let section = if section.len() > logs_cap {
            explained_cut(&section, logs_cap)
        } else {
            section
        };
        logs = nonempty(section);
    }

    let base = assemble(fixed, &logs, &None, &None);
    if !has_result_side {
        return base;
    }
    let side_cap = cap
        .saturating_sub(base.len())
        .saturating_sub(RESERVE_BYTES)
        .saturating_sub(if base.is_empty() { 0 } else { 2 });
    if side_cap == 0 {
        return base;
    }

    let result = if partial.is_some() && result_body.is_some() {
        let partial_cap = side_cap / 2;
        partial = nonempty(
            partial
                .take()
                .map(|section| explained_cut(&section, partial_cap))
                .unwrap_or_default(),
        );
        let used = partial
            .as_ref()
            .map(|section| section.len() + 2)
            .unwrap_or(0);
        result_body.and_then(|body| render_result(Some(body), side_cap.saturating_sub(used)))
    } else if let Some(section) = partial.take() {
        partial = nonempty(explained_cut(&section, side_cap));
        None
    } else {
        result_body.and_then(|body| render_result(Some(body), side_cap))
    };

    let mut text = assemble(fixed, &logs, &partial, &result);
    if text.len() > cap {
        text = explained_cut(&text, cap);
    }
    text
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
        let detail = error
            .message
            .lines()
            .next()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(|line| prefix_utf8(line, HEAD_MESSAGE_CAP));
        let mut line = format!(
            "run_code failed: uncaught ToolError {} from call #{} {} {} · program: failed · tool calls: {success} ok / {failed} failed · {duration}",
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
        let mut line = format!(
            "run_code failed: {label} · program: failed · tool calls: {success} ok / {failed} failed · {duration}"
        );
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
        format!(
            "run_code ok · program: ok · tool calls: {success} ok / {failed} failed · {duration}"
        )
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
    format!("… [truncated by byte cap: {omitted} bytes omitted of {total}] …")
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

fn nonempty(text: String) -> Option<String> {
    if text.is_empty() { None } else { Some(text) }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn text_until_tokens(target: usize, unit: &str) -> String {
        let mut out = String::new();
        while count_tokens(&out) < target {
            out.push_str(unit);
        }
        out
    }

    #[test]
    fn output_budgets_match_the_contract() {
        assert_eq!(RESULT_TOKEN_BUDGET, 10_000);
        assert_eq!(SUCCESS_LOGS_TOKEN_BUDGET, 3_000);
        assert_eq!(FAILURE_LOGS_TOKEN_BUDGET, 3_000);
        assert_eq!(RUN_CODE_VIEW_MAX_BYTES, 65_536);
    }

    #[test]
    fn partial_and_final_result_share_one_10k_budget() {
        let mut report = report();
        report.failure_kind = Some(FailureKind::Exception);
        report.failure_text = Some("boom".into());
        report.calls = (0..400)
            .map(|seq| {
                let mut call = snap(seq, "read", "path=large.txt", "ok", None);
                call.brief = Some(format!("{seq}: {}", "x".repeat(180)));
                call
            })
            .collect();
        report.value = Some(Value::String(format!(
            "{}RESULT_END_SENTINEL",
            "HEAD ".repeat(20_000)
        )));
        let text = render_model_view(&report);
        let result_area = text
            .split_once("completed tool calls before the failure:")
            .expect("partial result block")
            .1;
        assert!(
            crate::session::estimate::count_text_tokens(result_area) <= RESULT_TOKEN_BUDGET,
            "{}",
            crate::session::estimate::count_text_tokens(result_area)
        );
        assert!(result_area.contains("(token budget)"), "{result_area}");
        assert!(result_area.contains("result:\n"), "{result_area}");
        assert!(result_area.contains("RESULT_END_SENTINEL"), "{result_area}");
        assert!(text.len() <= RUN_CODE_VIEW_MAX_BYTES);
    }

    #[test]
    fn typical_3k_logs_plus_10k_result_fit_under_the_byte_cap() {
        let mut report = report();
        report.logs = (0..265)
            .map(|i| ("log".into(), format!("step {i} let value = compute();")))
            .collect();
        report.logs.push(("log".into(), "LOG_END_SENTINEL".into()));
        report.value = Some(Value::String(format!(
            "{}RESULT_END_SENTINEL",
            text_until_tokens(9_500, "let value = compute();\n")
        )));

        let text = render_model_view(&report);
        assert!(text.len() <= RUN_CODE_VIEW_MAX_BYTES, "{}", text.len());
        assert!(!text.contains("byte cap"), "{text}");
        let logs = text
            .split_once("\n\nlogs:")
            .expect("log section")
            .1
            .split("\n\nresult:")
            .next()
            .unwrap();
        assert!(logs.contains("LOG_END_SENTINEL"), "{logs}");
        assert!(!logs.contains("lines omitted"), "{logs}");
        let log_tokens = crate::session::estimate::count_text_tokens(&format!("logs:{logs}"));
        assert!(
            log_tokens > 2_500 && log_tokens <= SUCCESS_LOGS_TOKEN_BUDGET,
            "{log_tokens}"
        );
        let result = result_of(&text);
        assert!(result.contains("RESULT_END_SENTINEL"), "{result}");
        assert!(!result.contains("truncated"), "{result}");
        let result_tokens =
            crate::session::estimate::count_text_tokens(&format!("result:\n{result}"));
        assert!(
            result_tokens > 9_000 && result_tokens <= RESULT_TOKEN_BUDGET,
            "{result_tokens}"
        );
    }

    #[test]
    fn dropped_undefined_line_sits_in_the_head() {
        let mut report = report();
        report.value = Some(Value::Null);
        report.dropped_undefined = Some("dropped undefined at a.b, c[2]".into());
        // WP-210 H: a successful run does not mention dropped undefined.
        assert!(!render_model_view(&report).contains("dropped undefined"));
        report.failure_kind = Some(FailureKind::Exception);
        report.failure_text = Some("boom".into());
        let text = render_model_view(&report);
        let head = text.split("result:").next().unwrap_or("");
        assert!(head.contains("dropped undefined at a.b, c[2]"), "{text}");
    }

    #[test]
    fn coerced_non_finite_line_sits_in_the_success_head() {
        let mut report = report();
        report.value = Some(Value::Null);
        report.coerced_non_finite = Some("coerced non-finite number at n to null".into());
        let text = render_model_view(&report);
        let head = text.split("result:").next().unwrap_or("");
        assert!(
            head.contains("coerced non-finite number at n to null"),
            "{text}"
        );
    }

    #[test]
    fn large_string_result_keeps_head_and_tail_under_the_token_budget() {
        let mut report = report();
        let body = format!(
            "{}RESULT_END_SENTINEL",
            (0..20_000)
                .map(|i| format!("line {i} alpha beta\n"))
                .collect::<String>()
        );
        report.value = Some(Value::String(body));
        let text = render_model_view(&report);
        assert!(text.starts_with("run_code ok"));
        assert!(text.len() <= RUN_CODE_VIEW_MAX_BYTES, "{}", text.len());
        let result = text.split_once("result:\n").unwrap().1;
        assert!(
            result
                .lines()
                .next()
                .unwrap()
                .contains("exceeds 10.0k; truncated by token budget"),
            "{result}"
        );
        assert!(result.contains("line 0 alpha"), "{result}");
        assert!(result.contains("RESULT_END_SENTINEL"), "{result}");
        assert!(result.contains("tok omitted from the middle"), "{result}");
        assert!(!result.contains("byte cap"), "{result}");
        assert!(!text.starts_with("[blob:"));
    }

    #[test]
    fn default_view_stays_inside_the_budget_without_legacy_sections() {
        let mut report = report();
        report.failure_kind = Some(FailureKind::Timeout);
        report.failure_text = Some("x".repeat(20_000));
        report.duration_ms = 120_000;
        report.calls = (0..64)
            .map(|seq| snap(seq, "read", &"p".repeat(400), "ok", None))
            .collect();
        report.logs = (0..40)
            .map(|index| ("info".into(), format!("{index}{}", "L".repeat(800))))
            .collect();
        report.value = Some(Value::String("R".repeat(80_000)));
        let text = render_model_view(&report);
        assert!(text.len() <= RUN_CODE_VIEW_MAX_BYTES, "{}", text.len());
        assert!(
            text.contains("omitted") || text.contains("truncated"),
            "{text}"
        );
        assert!(!text.contains("diagnostics:"), "{text}");
        assert!(!text.contains("calls ("), "{text}");
        // Timeout label only — long failure_text must not flood the head.
        assert!(!text.contains(&"x".repeat(32)), "{text}");
        assert!(text.contains("timeout after"), "{text}");
    }

    #[test]
    fn success_header_counts_failures_from_call_states() {
        let mut report = report();
        report.calls = vec![
            snap(1, "read", "path=a", "ok", None),
            snap(2, "grep", "pattern=x", "error:not_found", Some("not_found")),
            snap(3, "grep", "pattern=y", "error:not_found", Some("not_found")),
        ];
        let text = render_model_view(&report);
        assert!(
            text.starts_with("run_code ok · program: ok · tool calls: 1 ok / 2 failed · 12ms"),
            "{text}"
        );
        assert!(!text.contains("rejected"), "{text}");
        assert!(!text.contains("calls ("), "{text}");
    }

    #[test]
    fn failure_header_counts_and_marks_unsettled() {
        let mut report = report();
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
            text.starts_with(
                "run_code failed: timeout after 120000ms · program: failed · tool calls: 3 ok / 0 failed · 120000ms"
            ),
            "{text}"
        );
        assert!(text.contains("1 calls were still running"), "{text}");
        assert!(!text.contains("calls ("), "{text}");
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

    fn result_of(text: &str) -> &str {
        text.split_once("result:\n").map(|(_, r)| r).unwrap_or("")
    }

    fn ok_with(value: Value) -> String {
        let mut report = report();
        report.value = Some(value);
        render_model_view(&report)
    }

    #[test]
    fn uniform_object_array_renders_as_a_table() {
        let text = ok_with(serde_json::json!([
            {"path": "a.rs", "line": 1, "hit": true},
            {"line": 2, "path": "b.rs", "hit": false},
            {"path": "c.rs", "hit": null, "line": 3.5},
        ]));
        assert_eq!(
            result_of(&text),
            // WP-210 E: columns follow the first row's key order (agent order).
            "[3]{path,line,hit}:\na.rs,1,true\nb.rs,2,false\nc.rs,3.5,"
        );
    }

    #[test]
    fn non_tabular_arrays_fall_back_to_compact_json() {
        let cases = [
            serde_json::json!([{"a": 1}, {"a": 2}]),
            serde_json::json!([{"a": 1}, {}, {"a": 3}]),
            serde_json::json!([{"a": [1]}, {"a": [2]}, {"a": [3]}]),
            serde_json::json!([{"a": {"x": 1}}, {"a": 2}, {"a": 3}]),
            serde_json::json!([1, 2, 3]),
        ];
        for value in cases {
            let text = ok_with(value.clone());
            assert_eq!(result_of(&text), serde_json::to_string(&value).unwrap());
        }
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
    fn other_values_are_compact_json_and_strings_verbatim() {
        let value = serde_json::json!({"a": {"b": [1, 2]}, "c": "d"});
        let text = ok_with(value.clone());
        assert_eq!(result_of(&text), serde_json::to_string(&value).unwrap());
        assert!(!result_of(&text).contains('\n'));
        let text = ok_with(Value::String("plain\n  text".into()));
        assert_eq!(result_of(&text), "plain\n  text");
    }

    #[test]
    fn cjk_result_is_cut_by_tokens_not_bytes_and_keeps_both_ends() {
        let mut body = "中文字符测试".repeat(5_000); // ~30k tok, ~90KB
        body.push_str("CJK_END_SENTINEL");
        let text = ok_with(Value::String(body));
        let result = result_of(&text);
        let notice = result.lines().next().unwrap();
        assert!(
            notice.starts_with("result ")
                && notice.contains("exceeds 10.0k; truncated by token budget"),
            "{notice}"
        );
        assert!(result.contains("CJK_END_SENTINEL"), "{result}");
        assert!(result.contains("tok omitted from the middle"), "{result}");
        let result_section = format!("result:\n{result}");
        assert!(
            crate::session::estimate::count_text_tokens(&result_section) <= RESULT_TOKEN_BUDGET,
            "{}",
            crate::session::estimate::count_text_tokens(&result_section)
        );
        assert!(text.len() <= RUN_CODE_VIEW_MAX_BYTES);
    }

    #[test]
    fn byte_fallback_keeps_the_tail_and_names_the_byte_cap() {
        // Few tokens, many bytes: long runs compress well in cl100k.
        let text = ok_with(Value::String(format!(
            "{}BYTE_END_SENTINEL",
            " ".repeat(400_000)
        )));
        assert!(text.len() <= RUN_CODE_VIEW_MAX_BYTES, "{}", text.len());
        assert!(text.contains("truncated by byte cap"), "{text}");
        assert!(text.contains("BYTE_END_SENTINEL"), "{text}");
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
    fn notice_appears_only_when_over_budget() {
        let text = ok_with(serde_json::json!({"small": true}));
        assert!(!text.contains("exceeds"), "{text}");
        assert!(!text.contains("truncated"), "{text}");
    }

    #[test]
    fn success_logs_keep_the_head_within_3k_tokens() {
        let mut report = report();
        report.logs = (0..7)
            .map(|i| ("log".into(), format!("line {i}")))
            .collect();
        report.value = Some(Value::Null);
        let text = render_model_view(&report);
        assert!(text.contains("\n\nlogs:\n[log] line 0\n"), "{text}");
        assert!(text.contains("[log] line 6\n\nresult:"), "{text}");
        assert!(!text.contains("omitted"), "{text}");

        report.logs = (0..2_000)
            .map(|i| ("log".into(), format!("step {i} doing some work here")))
            .collect();
        let text = render_model_view(&report);
        let logs = text
            .split_once("\n\nlogs:")
            .unwrap()
            .1
            .split("\n\nresult:")
            .next()
            .unwrap();
        assert!(logs.starts_with("\n[log] step 0 doing"), "{logs}");
        let tail = logs.lines().last().unwrap();
        assert!(
            tail.starts_with("… ") && tail.ends_with(" lines omitted (token budget)"),
            "{tail}"
        );
        let whole = format!("logs:{logs}");
        assert!(
            crate::session::estimate::count_text_tokens(&whole) <= SUCCESS_LOGS_TOKEN_BUDGET,
            "{}",
            crate::session::estimate::count_text_tokens(&whole)
        );
    }

    #[test]
    fn failed_run_keeps_completed_calls_and_separates_program_from_tool_counts() {
        let mut report = report();
        report.failure_kind = Some(FailureKind::Exception);
        report.failure_text = Some("Cannot read properties of null".into());
        report.failure_location = Some(SourceLocation {
            line: 3,
            column: 14,
        });
        report.source =
            "const a = await tools.file.read({path:'a'});\nconst b = 2;\nreturn null.x;".into();
        report.failure_stack = Some(
            "    at boom (program.js:4:14)\n    at <anonymous> (libs.js:9:1)\n    at <eval> (program.js:1:1)".into(),
        );
        let mut first = snap(1, "read", "path=a", "ok", None);
        first.brief = Some("{\"ok\":true,\"lines\":[\"fn a\"]}".into());
        let mut second = snap(
            2,
            "grep",
            "pattern=x",
            "error:invalid_args",
            Some("invalid_args"),
        );
        second.brief = Some("bad regex".into());
        report.calls = vec![first, second];
        let text = render_model_view(&report);
        let head = text.lines().next().unwrap();
        assert!(
            head.starts_with(
                "run_code failed: exception · program: failed · tool calls: 1 ok / 1 failed"
            ),
            "{head}"
        );
        assert!(
            head.contains("line 3, column 14 · return null.x;"),
            "{head}"
        );
        assert!(text.contains("stack:\n  at boom (line 3:14)"), "{text}");
        assert!(!text.contains("libs.js"), "{text}");
        assert!(!text.contains("<eval>"), "{text}");
        assert!(
            text.contains(
                "completed tool calls before the failure:\n#1 read path=a → ok: {\"ok\":true"
            ),
            "{text}"
        );
        assert!(
            text.contains("#2 grep pattern=x → failed(invalid_args): bad regex"),
            "{text}"
        );
        // A successful run never prints the partial-results block.
        report.failure_kind = None;
        assert!(!render_model_view(&report).contains("completed tool calls"));
    }

    #[test]
    fn short_stack_drops_eval_frames_by_name() {
        let mut report = report();
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
        assert!(crate::session::estimate::count_text_tokens(body) <= RESULT_TOKEN_BUDGET);
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
        let value = serde_json::json!({
            "catalog": {"path": "src/apps/catalog.rs", "n": 660, "text": "x".repeat(20_000)},
            "app_panel": {"path": "web/src/lib/appPanel.ts", "n": 420, "text": "x".repeat(20_000)},
            "app_tab": {"path": "web/src/dockview/tabs/AppTab.tsx", "n": 310, "text": "x".repeat(20_000)},
            "app_view": {"path": "web/src/dockview/panels/AppViewPanel.tsx", "n": 280, "text": "x".repeat(20_000)},
        });
        let text = ok_with(value);
        let result = result_of(&text);
        let (notice, body) = result.split_once('\n').unwrap();
        assert!(notice.contains("exceeds 10.0k; truncated"), "{notice}");
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
        assert!(crate::session::estimate::count_text_tokens(body) <= RESULT_TOKEN_BUDGET);
    }

    #[test]
    fn failure_logs_keep_the_tail_within_3k_tokens() {
        let mut report = report();
        report.failure_kind = Some(FailureKind::Exception);
        report.failure_text = Some("boom".into());
        report.logs = (0..2_000)
            .map(|i| ("log".into(), format!("step {i} doing work here")))
            .collect();
        let text = render_model_view(&report);
        let logs = text.split_once("logs:").unwrap().1;
        assert!(
            crate::session::estimate::count_text_tokens(&format!("logs:{logs}"))
                <= FAILURE_LOGS_TOKEN_BUDGET
        );
        assert!(logs.contains("step 1999 doing work here"), "{logs}");
        assert!(!logs.contains("step 0 doing"), "{logs}");
        assert!(
            logs.contains("earlier lines omitted] (token budget)"),
            "{logs}"
        );
        let mut small = super::tests::report();
        small.failure_kind = Some(FailureKind::Exception);
        small.logs = vec![("log".into(), "only".into())];
        assert!(render_model_view(&small).contains("logs:\n[log] only"));
    }

    #[test]
    fn human_data_stays_structured_for_the_pretty_frontend() {
        let mut report = report();
        report.value = Some(serde_json::json!([{"a": 1}, {"a": 2}, {"a": 3}]));
        report.logs = vec![("log".into(), "x".into())];
        let data = human_data(&report).unwrap();
        assert!(data.get("succeeded").is_some() && data.get("by_tool").is_some());
        assert_eq!(data["ok"], Value::Bool(true));
    }
}

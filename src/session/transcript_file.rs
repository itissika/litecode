//! Canonical read-only session transcript projection.
//!
//! One virtual markdown-like file per session, addressed as
//! `.litecode/sessions/<full_session_id>.md`. SQLite remains the only source of
//! truth; this module never writes files.
//!
//! # Addressing contract
//!
//! A physical line holds at most [`RENDER_WIDTH`] chars of one item's text, cut
//! at deterministic char offsets (hard cut, no word preference). A rendered
//! `L<n>` therefore addresses a *bounded* line: no line is ever an entire
//! fifty-thousand-char row, and one item can span as many lines as it needs.
//!
//! [`LineSpan::char_start_in_item`] / [`LineSpan::char_end_in_item`] give the
//! item-local char range each line covers. The ranges stay contiguous and cover
//! the item text exactly once (the line that carries the newline owns it), so a
//! hit's `char_start` resolves to the very line that contains it — `read`,
//! `grep` and `session_search` all address the same rendering.
//!
//! [`RENDER_WIDTH`] is an addressing constant, not a display preference: any
//! change renumbers every line of every session at once. Never make it a
//! per-call parameter.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::session::data::read_bytes;
use crate::tool::output::{BLOB_PREFIX, blob_dir};
use crate::types::{Item, LitecodeError, Result, item_text_preview};

pub const VIRTUAL_SESSION_DIR: &str = ".litecode/sessions";
pub const VIRTUAL_SESSION_PREFIX: &str = ".litecode/sessions/";
pub const VIRTUAL_SESSION_SUFFIX: &str = ".md";

/// Physical render width of the virtual transcript, in chars (hard cut).
///
/// Bounds every addressable line so one hit is one readable fragment instead of
/// a whole spilled row. Changing it renumbers the `L<n>` of every session, so it
/// is a product constant — never a per-call option.
pub const RENDER_WIDTH: usize = 160;
pub const READ_ONLY_MSG: &str =
    "this path is a read-only session transcript projection; use read or grep";
pub const IN_CONTEXT_WINDOW_MSG: &str =
    "requested lines are already in the current session context window";

pub const SEARCHABLE_KINDS: &[&str] = &[
    "item/user",
    "item/assistant",
    "item/tool_call",
    "item/tool_result",
];

#[derive(Debug, Clone)]
pub struct SearchableRow {
    pub session_id: String,
    pub seq: i64,
    pub kind: String,
    pub item_type: String,
    pub body: Option<String>,
    pub body_ref: Option<String>,
}

#[derive(Debug, Clone)]
pub struct LineSpan {
    pub line: u32,
    pub seq: i64,
    pub kind: String,
    pub item_type: String,
    pub is_header: bool,
    /// Inclusive char offset of this body line inside the item plain text.
    pub char_start_in_item: usize,
    /// Exclusive char offset of this body line inside the item plain text.
    pub char_end_in_item: usize,
}

#[derive(Debug, Clone)]
pub struct TranscriptFile {
    pub session_id: String,
    pub virtual_path: String,
    pub lines: Vec<String>,
    pub line_index: Vec<LineSpan>,
    /// Tool name per `seq`, for the rows that are a tool call or a tool result.
    /// A result carries the name of the call it answers, resolved inside its own
    /// session; rows that are neither, or whose call is not in this file, are
    /// absent rather than empty.
    pub tool_names: HashMap<i64, String>,
    /// Call id of every `tool_result` row in this file, keyed by the row's seq.
    pub result_call_ids: HashMap<i64, String>,
    /// Call ids whose call reads the session store. A result answering one of
    /// these is an echo copy: the corpus drops it, and a hit served from a stale
    /// index has to be dropped for the same reason at read time.
    pub session_read_calls: HashSet<String>,
}

impl TranscriptFile {
    pub fn total_lines(&self) -> usize {
        self.lines.len()
    }

    pub fn line_text(&self, line: u32) -> Option<&str> {
        self.lines
            .get(line.saturating_sub(1) as usize)
            .map(String::as_str)
    }

    pub fn seq_at(&self, line: u32) -> Option<i64> {
        self.span_at(line).map(|s| s.seq)
    }

    pub fn span_at(&self, line: u32) -> Option<&LineSpan> {
        if line == 0 {
            return None;
        }
        self.line_index.get(line.saturating_sub(1) as usize)
    }

    /// The tool a row is a call of (or the result of), when it is one.
    pub fn tool_name(&self, seq: i64) -> Option<&str> {
        self.tool_names.get(&seq).map(String::as_str)
    }

    /// True when this `tool_result` seq is an echo copy of session content.
    ///
    /// Answered from the parse the renderer already did, so a search hit can be
    /// re-checked against the live store for free. That is what keeps "the index
    /// is behind" from ever meaning "a copy is searchable again": staleness may
    /// cost recall, never truth.
    pub fn is_echo_result(&self, seq: i64) -> bool {
        self.result_call_ids
            .get(&seq)
            .is_some_and(|call_id| self.session_read_calls.contains(call_id))
    }

    pub fn first_body_line(&self, seq: i64) -> Option<u32> {
        self.line_index
            .iter()
            .find(|s| s.seq == seq && !s.is_header)
            .map(|s| s.line)
    }

    /// Map an item-local char offset to a physical body line.
    pub fn line_for_char(&self, seq: i64, char_start: usize) -> Option<u32> {
        let mut last_body: Option<u32> = None;
        for span in &self.line_index {
            if span.seq != seq || span.is_header {
                continue;
            }
            last_body = Some(span.line);
            if char_start < span.char_end_in_item {
                return Some(span.line);
            }
        }
        last_body
    }

    pub fn line_for_hit(&self, seq: i64, char_start: usize, char_end: usize) -> Option<u32> {
        if char_start == 0 && char_end == 0 {
            return self.first_body_line(seq);
        }
        self.line_for_char(seq, char_start)
            .or_else(|| self.first_body_line(seq))
    }
}

pub fn virtual_path_for(session_id: &str) -> String {
    format!("{VIRTUAL_SESSION_PREFIX}{session_id}{VIRTUAL_SESSION_SUFFIX}")
}

fn normalize_virtual_rel(raw: &str) -> String {
    let normalized = raw.trim().replace('\\', "/");
    let stripped = normalized
        .strip_prefix("./")
        .unwrap_or(normalized.as_str())
        .trim_start_matches('/');
    stripped.trim_end_matches('/').to_string()
}

/// True when `raw` is the virtual session directory (not a single file).
pub fn is_virtual_session_dir(raw: &str) -> bool {
    let stripped = normalize_virtual_rel(raw);
    if stripped.contains("..") || stripped.contains('*') || stripped.contains('?') {
        return false;
    }
    stripped == VIRTUAL_SESSION_DIR
}

/// Parse a workspace-relative virtual session path. Returns the session id stem
/// (not yet resolved against the DB).
pub fn try_parse_virtual_path(raw: &str) -> Option<String> {
    let stripped = normalize_virtual_rel(raw);
    if stripped.contains("..") || stripped.contains('*') || stripped.contains('?') {
        return None;
    }
    let rest = stripped.strip_prefix(VIRTUAL_SESSION_PREFIX)?;
    if rest.contains('/') {
        return None;
    }
    let stem = rest.strip_suffix(VIRTUAL_SESSION_SUFFIX)?;
    if stem.is_empty() || stem.contains(['/', '\\']) {
        return None;
    }
    Some(stem.to_string())
}

pub fn is_virtual_session_path(raw: &str) -> bool {
    try_parse_virtual_path(raw).is_some()
}

/// Canonical virtual paths for every session id.
pub fn list_virtual_paths(ids: impl IntoIterator<Item = impl AsRef<str>>) -> Vec<String> {
    let mut out: Vec<String> = ids
        .into_iter()
        .map(|id| id.as_ref().to_string())
        .filter(|id| !id.is_empty())
        .map(|id| virtual_path_for(&id))
        .collect();
    out.sort();
    out
}

pub fn row_plain_text(row: &SearchableRow, data_root: &Path) -> Result<Option<String>> {
    match row_plain_text_strict(row, data_root) {
        Ok(text) => Ok(text),
        Err(e) => {
            tracing::warn!(
                session_id = %row.session_id,
                seq = row.seq,
                error = %e,
                "session transcript skip unreadable row"
            );
            Ok(None)
        }
    }
}

/// Read one row's projected text, treating an unreadable source as an error.
///
/// [`row_plain_text`] answers "what can I show?" and drops whatever it cannot
/// read. The index cannot afford that answer: a dropped row next to an advanced
/// cursor is a document that is permanently missing from search. The derive
/// pipeline uses this instead, so the whole batch fails and the cursor stays
/// where it was.
pub fn row_plain_text_strict(row: &SearchableRow, data_root: &Path) -> Result<Option<String>> {
    let Some(item) = row_item(row, data_root)? else {
        return Ok(None);
    };
    let text = item_text_preview(&item);
    if text.trim().is_empty() {
        Ok(None)
    } else {
        Ok(Some(normalize_newlines(&text)))
    }
}

/// One row's parsed item, when it has a readable body.
///
/// The projected text and the tool linkage are both read off this one parse, so
/// rendering a transcript parses each row exactly once.
fn row_item(row: &SearchableRow, data_root: &Path) -> Result<Option<Item>> {
    let json = if let Some(body) = &row.body {
        body.clone()
    } else if let Some(body_ref) = &row.body_ref {
        load_blob_text(body_ref, data_root).map_err(|e| {
            LitecodeError::SessionStorage(format!(
                "unreadable transcript blob for {}:{}: {e}",
                row.session_id, row.seq
            ))
        })?
    } else {
        return Ok(None);
    };
    if let Ok(item) = serde_json::from_str::<Item>(&json) {
        return Ok(Some(item));
    }
    if let Ok(body) = serde_json::from_str::<crate::session::model::CompactedBody>(&json) {
        return Ok(Some(body.agent_item()));
    }
    Err(LitecodeError::SessionStorage(format!(
        "unparseable transcript row {}:{}",
        row.session_id, row.seq
    )))
}

/// Whether a call reads the session store: a `session_search`, or a `read` /
/// `grep` whose arguments point into it.
///
/// The single rule for "is this call an echo call", shared by the index-time
/// echo closure (`echo.rs`) and the read-time check — so the two can never
/// disagree about what an echo is. Detection is structural (name plus arguments),
/// not textual: a content signature catches only part of the copies and misfires
/// on bash/glob output.
pub fn call_reads_sessions(name: &str, arguments: Option<&str>) -> bool {
    if name == "session_search" {
        return true;
    }
    if !matches!(name, "read" | "grep") {
        return false;
    }
    let Some(raw) = arguments else {
        return false;
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(raw) else {
        return false;
    };
    let mut found = Vec::new();
    collect_strings(&v, &mut found);
    found.iter().any(|s| is_session_target(s))
}

fn collect_strings<'a>(v: &'a serde_json::Value, out: &mut Vec<&'a str>) {
    match v {
        serde_json::Value::String(s) => out.push(s),
        serde_json::Value::Array(a) => a.iter().for_each(|x| collect_strings(x, out)),
        serde_json::Value::Object(m) => m.values().for_each(|x| collect_strings(x, out)),
        _ => {}
    }
}

/// `.litecode/sessions` followed by `/` (a file below it) or end (the directory
/// itself). `.litecode/sessions.db` is *not* a hit. Backslashes are folded.
fn is_session_target(arg: &str) -> bool {
    let norm = arg.replace('\\', "/");
    let Some(pos) = norm.find(VIRTUAL_SESSION_DIR) else {
        return false;
    };
    let rest = &norm[pos + VIRTUAL_SESSION_DIR.len()..];
    rest.is_empty() || rest.starts_with('/')
}

pub fn load_blob_text(body_ref: &str, data_root: &Path) -> Result<String> {
    let rest = body_ref
        .strip_prefix(BLOB_PREFIX)
        .ok_or_else(|| LitecodeError::Config(format!("invalid body_ref: {body_ref}")))?;
    let (id, _) = rest
        .split_once(']')
        .ok_or_else(|| LitecodeError::Config(format!("invalid body_ref: {body_ref}")))?;
    match read_bytes(data_root, id) {
        Ok(bytes) => String::from_utf8(bytes)
            .map_err(|e| LitecodeError::SessionStorage(format!("blob {id} is not utf-8: {e}"))),
        Err(_) => {
            let blob_path = blob_dir(data_root).join(format!("{id}.txt"));
            std::fs::read_to_string(blob_path).map_err(Into::into)
        }
    }
}

pub fn iter_searchable_texts(
    rows: &[SearchableRow],
    data_root: &Path,
) -> Result<Vec<(String, i64, String, String)>> {
    let mut out = Vec::new();
    for row in rows {
        let Some(text) = row_plain_text(row, data_root)? else {
            continue;
        };
        out.push((row.session_id.clone(), row.seq, row.item_type.clone(), text));
    }
    Ok(out)
}

pub fn load_transcript_file(
    session_id: &str,
    rows: &[SearchableRow],
    data_root: &Path,
) -> Result<TranscriptFile> {
    let virtual_path = virtual_path_for(session_id);
    let mut lines = Vec::new();
    let mut line_index = Vec::new();
    // `(seq, call_id, name)` of the tool rows, kept apart from the render loop
    // because a result resolves to a call that may only be read later on.
    let mut links: Vec<(i64, String, Option<String>)> = Vec::new();
    let mut result_call_ids: HashMap<i64, String> = HashMap::new();
    let mut session_read_calls: HashSet<String> = HashSet::new();
    for row in rows {
        if row.session_id != session_id {
            continue;
        }
        let item = match row_item(row, data_root) {
            Ok(Some(item)) => item,
            Ok(None) => continue,
            Err(e) => {
                tracing::warn!(
                    session_id = %row.session_id,
                    seq = row.seq,
                    error = %e,
                    "session transcript skip unreadable row"
                );
                continue;
            }
        };
        let text = item_text_preview(&item);
        if text.trim().is_empty() {
            continue;
        }
        match &item {
            // `(call_id, name)` of a tool call: the link a later result resolves
            // against, plus the echo rule's own input.
            Item::FunctionCall(call) => {
                if call_reads_sessions(&call.name, Some(call.arguments.as_str())) {
                    session_read_calls.insert(call.call_id.clone());
                }
                links.push((row.seq, call.call_id.clone(), Some(call.name.clone())));
            }
            // A result does not name its tool, only the call it answers — and it
            // is the only row that can be an echo copy.
            Item::FunctionCallOutput(out) => {
                result_call_ids.insert(row.seq, out.call_id.clone());
                links.push((row.seq, out.call_id.clone(), None));
            }
            _ => {}
        }
        push_item(
            &mut lines,
            &mut line_index,
            row.seq,
            &row.kind,
            &row.item_type,
            &normalize_newlines(&text),
        );
    }
    let call_names: HashMap<&str, &str> = links
        .iter()
        .filter_map(|(_, call_id, name)| Some((call_id.as_str(), name.as_deref()?)))
        .collect();
    let tool_names: HashMap<i64, String> = links
        .iter()
        .filter_map(|(seq, call_id, name)| {
            let name = name
                .as_deref()
                .or_else(|| call_names.get(call_id.as_str()).copied())?;
            Some((*seq, name.to_string()))
        })
        .collect();
    Ok(TranscriptFile {
        session_id: session_id.to_string(),
        virtual_path,
        lines,
        line_index,
        tool_names,
        result_call_ids,
        session_read_calls,
    })
}

fn push_item(
    lines: &mut Vec<String>,
    index: &mut Vec<LineSpan>,
    seq: i64,
    kind: &str,
    item_type: &str,
    plain: &str,
) {
    let header = format!("[s{seq} {}]", type_label(kind, item_type));
    push_line(
        lines,
        index,
        LineSpan {
            line: 0,
            seq,
            kind: kind.to_string(),
            item_type: item_type.to_string(),
            is_header: true,
            char_start_in_item: 0,
            char_end_in_item: 0,
        },
        header,
    );

    let total_chars = plain.chars().count();
    let mut offset = 0usize;
    let body_lines: Vec<&str> = plain.lines().collect();
    for (i, body) in body_lines.iter().enumerate() {
        let line_chars = body.chars().count();
        let mut line_end = offset + line_chars;
        let has_more = i + 1 < body_lines.len() || plain.ends_with('\n');
        if has_more && line_end < total_chars {
            line_end += 1;
        }
        if line_end < offset {
            line_end = offset;
        }
        // Hard cut at `RENDER_WIDTH`. The segments of one body line stay
        // contiguous and the last one owns the trailing newline, so the whole
        // item text is covered exactly once and `line_for_char` is exact.
        let chars: Vec<char> = body.chars().collect();
        let mut seg_start = 0usize;
        loop {
            let seg_end = (seg_start + RENDER_WIDTH).min(chars.len());
            let is_last = seg_end == chars.len();
            let start = offset + seg_start;
            let end = if is_last { line_end } else { offset + seg_end };
            push_line(
                lines,
                index,
                LineSpan {
                    line: 0,
                    seq,
                    kind: kind.to_string(),
                    item_type: item_type.to_string(),
                    is_header: false,
                    char_start_in_item: start,
                    char_end_in_item: end.max(start),
                },
                chars[seg_start..seg_end].iter().collect(),
            );
            if is_last {
                break;
            }
            seg_start = seg_end;
        }
        offset = line_end;
    }
}

/// Reader-facing label for one row — the same vocabulary the search view uses.
///
/// The mapped cases cover every kind the store writes today; anything else falls
/// back to its raw `kind`/`item_type` so a type added ahead of this table still
/// renders instead of disappearing.
pub fn type_label(kind: &str, item_type: &str) -> String {
    match (kind, item_type) {
        ("item/user", _) => "user".into(),
        ("item/assistant", "reasoning") => "assistant reasoning".into(),
        ("item/assistant", _) => "assistant message".into(),
        ("item/tool_call", _) => "tool call".into(),
        ("item/tool_result", _) => "tool result".into(),
        ("compacted", _) => "compacted summary".into(),
        _ => format!("{kind} {item_type}"),
    }
}

fn push_line(lines: &mut Vec<String>, index: &mut Vec<LineSpan>, mut span: LineSpan, text: String) {
    span.line = (lines.len() + 1) as u32;
    lines.push(text);
    index.push(span);
}

fn normalize_newlines(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{SessionData, WorkspaceWriteLease};
    use crate::types::user_text;
    use std::sync::Arc;
    use tempfile::TempDir;

    fn seeded(items: &[crate::types::Item]) -> (Arc<SessionData>, String) {
        let data = SessionData::open_ephemeral().unwrap();
        let session_id = data.create_session("/proj", "default", None).unwrap();
        data.insert_items(&session_id, items).unwrap();
        (data, session_id)
    }

    #[test]
    fn parse_virtual_path_accepts_full_id_and_normalizes_slashes() {
        assert_eq!(
            try_parse_virtual_path(".litecode/sessions/01ABCDEF.md").as_deref(),
            Some("01ABCDEF")
        );
        assert_eq!(
            try_parse_virtual_path(".litecode\\sessions\\01ABCDEF.md").as_deref(),
            Some("01ABCDEF")
        );
        assert_eq!(
            try_parse_virtual_path("./.litecode/sessions/01ABCDEF.md").as_deref(),
            Some("01ABCDEF")
        );
        assert!(try_parse_virtual_path(".litecode/sessions/../x.md").is_none());
        assert!(try_parse_virtual_path(".litecode/sessions/*.md").is_none());
        assert!(try_parse_virtual_path(".litecode/sessions/a/b.md").is_none());
        assert!(try_parse_virtual_path("sessions/01ABCDEF.md").is_none());
        assert_eq!(
            virtual_path_for("01ABCDEF"),
            ".litecode/sessions/01ABCDEF.md"
        );
        assert!(is_virtual_session_dir(".litecode/sessions"));
        assert!(is_virtual_session_dir(".litecode/sessions/"));
        assert!(is_virtual_session_dir(".litecode\\sessions"));
        assert!(!is_virtual_session_dir(".litecode/sessions/01ABCDEF.md"));
        assert!(!is_virtual_session_dir(".litecode"));
        assert!(!is_virtual_session_dir("src"));
    }

    #[test]
    fn projection_maps_multiline_body_and_char_offsets() {
        let (data, sid) = seeded(&[user_text("alpha\nbeta NEEDLE gamma\ndelta")]);
        let reader = data.reader();
        let rows = reader.searchable_rows_blocking(Some(&sid)).unwrap();
        let file = load_transcript_file(&sid, &rows, reader.data_root()).unwrap();
        assert_eq!(file.virtual_path, virtual_path_for(&sid));
        assert!(file.lines[0].starts_with("[s0 user]"));
        assert_eq!(file.lines[1], "alpha");
        assert_eq!(file.lines[2], "beta NEEDLE gamma");
        assert_eq!(file.lines[3], "delta");

        let needle_start = "alpha\nbeta NEEDLE gamma\ndelta".find("NEEDLE").unwrap();
        let line = file.line_for_char(0, needle_start).unwrap();
        assert_eq!(line, 3);
        assert_eq!(file.seq_at(3), Some(0));
        assert_eq!(file.line_for_hit(0, 0, 0), Some(2));
    }

    #[test]
    fn empty_rows_are_skipped() {
        let (data, sid) = seeded(&[user_text("   "), user_text("kept UNIQUE")]);
        let reader = data.reader();
        let rows = reader.searchable_rows_blocking(Some(&sid)).unwrap();
        let file = load_transcript_file(&sid, &rows, reader.data_root()).unwrap();
        let headers: Vec<_> = file
            .line_index
            .iter()
            .filter(|s| s.is_header)
            .map(|s| s.seq)
            .collect();
        assert_eq!(headers, vec![1]);
    }

    #[test]
    fn tool_rows_name_the_tool_they_call() {
        use crate::authority::responses::{
            FunctionCallOutput, FunctionCallOutputItemParam, FunctionToolCall,
        };
        use crate::types::Item;

        let (data, sid) = seeded(&[
            user_text("hello"),
            Item::FunctionCall(FunctionToolCall {
                arguments: r#"{"command":"ls"}"#.into(),
                call_id: "call_1".into(),
                namespace: None,
                name: "bash".into(),
                id: None,
                status: None,
            }),
            Item::FunctionCallOutput(FunctionCallOutputItemParam {
                call_id: "call_1".into(),
                output: FunctionCallOutput::Text("ok".into()),
                id: None,
                status: None,
            }),
        ]);
        let reader = data.reader();
        let rows = reader.searchable_rows_blocking(Some(&sid)).unwrap();
        let file = load_transcript_file(&sid, &rows, reader.data_root()).unwrap();

        assert_eq!(file.tool_name(0), None, "a message is not a tool row");
        assert_eq!(file.tool_name(1), Some("bash"), "a call names its tool");
        assert_eq!(file.tool_name(2), Some("bash"), "a result names its call");
    }

    /// The read-time echo check: a hit served from a stale index must not make a
    /// copy searchable again. It is answered from the same parse the renderer did.
    #[test]
    fn a_result_answering_a_session_read_is_an_echo_at_read_time() {
        use crate::authority::responses::{
            FunctionCallOutput, FunctionCallOutputItemParam, FunctionToolCall,
        };
        use crate::types::Item;

        let call = |name: &str, arguments: &str, call_id: &str| {
            Item::FunctionCall(FunctionToolCall {
                arguments: arguments.into(),
                call_id: call_id.into(),
                namespace: None,
                name: name.into(),
                id: None,
                status: None,
            })
        };
        let result = |call_id: &str| {
            Item::FunctionCallOutput(FunctionCallOutputItemParam {
                call_id: call_id.into(),
                output: FunctionCallOutput::Text("copied page".into()),
                id: None,
                status: None,
            })
        };

        let (data, sid) = seeded(&[
            call("session_search", r#"{"query":"auth"}"#, "c1"),
            result("c1"),
            call("bash", r#"{"command":"ls"}"#, "c2"),
            result("c2"),
            call("read", r#"{"file_path":".litecode/sessions/01ARZ.md"}"#, "c3"),
            result("c3"),
        ]);
        let reader = data.reader();
        let rows = reader.searchable_rows_blocking(Some(&sid)).unwrap();
        let file = load_transcript_file(&sid, &rows, reader.data_root()).unwrap();

        assert!(file.is_echo_result(1), "a session_search result is a copy");
        assert!(!file.is_echo_result(3), "a bash result is ordinary content");
        assert!(
            file.is_echo_result(5),
            "a read into the session store is a copy"
        );
        assert!(
            !file.is_echo_result(0),
            "a call row is the intent, never the copy"
        );
    }

    #[test]
    fn the_session_read_rule_reads_name_and_arguments_only() {
        assert!(call_reads_sessions(
            "session_search",
            Some(r#"{"query":"x"}"#)
        ));
        assert!(call_reads_sessions(
            "read",
            Some(r#"{"file_path":".litecode/sessions/01ARZ.md"}"#)
        ));
        assert!(call_reads_sessions(
            "grep",
            Some(r#"{"pattern":"x","path":".litecode/sessions"}"#)
        ));
        assert!(!call_reads_sessions(
            "read",
            Some(r#"{"file_path":"src/main.rs"}"#)
        ));
        assert!(!call_reads_sessions(
            "bash",
            Some(r#"{"command":"ls .litecode/sessions"}"#)
        ));
        assert!(!call_reads_sessions("read", None));
    }

    #[test]
    fn session_target_matching() {
        assert!(is_session_target(
            ".litecode/sessions/01ARZ3NDEKTSV4RRFFQ69G5FAV.md"
        ));
        assert!(is_session_target(
            r"E:\ws\.litecode\sessions\01ARZ3NDEKTSV4RRFFQ69G5FAV.md"
        ));
        assert!(is_session_target(".litecode/sessions"));
        assert!(!is_session_target(".litecode/sessions.db"));
        assert!(!is_session_target("src/sessions/mod.rs"));
    }

    #[test]
    fn compacted_rows_are_projected() {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("sessions.db");
        let lease = WorkspaceWriteLease::acquire(dir.path()).unwrap();
        let data = SessionData::open(&lease, &db).unwrap();
        let sid = data.create_session("/proj", "default", None).unwrap();
        data.insert_items(&sid, &[user_text("old"), user_text("live")])
            .unwrap();
        data.compact_from(&sid, &user_text("summary compact body"), Some(1), 3)
            .unwrap();
        let reader = data.reader();
        let rows = reader.searchable_rows_blocking(Some(&sid)).unwrap();
        let file = load_transcript_file(&sid, &rows, reader.data_root()).unwrap();
        let kinds: Vec<_> = file
            .line_index
            .iter()
            .filter(|s| s.is_header)
            .map(|s| s.kind.as_str())
            .collect();
        assert!(kinds.contains(&"compacted"));
        assert!(
            file.lines
                .iter()
                .any(|l| l.contains("summary compact body"))
        );
    }

    #[test]
    fn blob_rows_round_trip_into_projection() {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("sessions.db");
        let lease = WorkspaceWriteLease::acquire(dir.path()).unwrap();
        let data = SessionData::open(&lease, &db).unwrap();
        let session_id = data.create_session("/proj", "default", None).unwrap();
        let data_root = db.parent().unwrap().to_path_buf();
        let blobs = blob_dir(&data_root);
        std::fs::create_dir_all(&blobs).unwrap();
        std::fs::write(blobs.join("abc.txt"), r#"{"type":"message","role":"user","content":[{"type":"input_text","text":"BLOB_NEEDLE here"}]}"#).unwrap();
        // Direct SQL insert of a blob-backed user item is heavy; instead verify
        // load_blob_text + row_plain_text which the projection uses.
        let row = SearchableRow {
            session_id,
            seq: 9,
            kind: "item/user".into(),
            item_type: "message".into(),
            body: None,
            body_ref: Some("[blob:abc]".into()),
        };
        let text = row_plain_text(&row, &data_root).unwrap().unwrap();
        assert!(text.contains("BLOB_NEEDLE"));
    }

    #[test]
    fn long_body_line_is_wrapped_at_the_render_width() {
        let long = "x".repeat(RENDER_WIDTH * 2 + 17);
        let (data, sid) = seeded(&[user_text(&long)]);
        let reader = data.reader();
        let rows = reader.searchable_rows_blocking(Some(&sid)).unwrap();
        let file = load_transcript_file(&sid, &rows, reader.data_root()).unwrap();
        // header + 160 + 160 + 17
        assert_eq!(file.lines.len(), 4);
        assert!(file.lines.iter().all(|l| l.chars().count() <= RENDER_WIDTH));
        assert_eq!(file.lines[1].chars().count(), RENDER_WIDTH);
        assert_eq!(file.lines[2].chars().count(), RENDER_WIDTH);
        assert_eq!(file.lines[3].chars().count(), 17);
        // A match resolves to the segment that contains it, not to the row head.
        assert_eq!(file.line_for_char(0, 5), Some(2));
        assert_eq!(file.line_for_char(0, RENDER_WIDTH), Some(3));
        assert_eq!(file.line_for_char(0, RENDER_WIDTH * 2 + 3), Some(4));
        assert_eq!(
            file.line_for_hit(0, RENDER_WIDTH * 2 + 3, RENDER_WIDTH * 2 + 9),
            Some(4)
        );
    }

    #[test]
    fn wrapped_segments_cover_the_item_text_exactly_once() {
        let text = "abcdefghij".repeat(70);
        let (data, sid) = seeded(&[user_text(&text)]);
        let reader = data.reader();
        let rows = reader.searchable_rows_blocking(Some(&sid)).unwrap();
        let file = load_transcript_file(&sid, &rows, reader.data_root()).unwrap();
        let spans: Vec<_> = file
            .line_index
            .iter()
            .filter(|s| s.seq == 0 && !s.is_header)
            .collect();
        assert_eq!(spans.len(), 5);
        assert_eq!(spans[0].char_start_in_item, 0);
        for pair in spans.windows(2) {
            assert_eq!(pair[0].char_end_in_item, pair[1].char_start_in_item);
        }
        assert_eq!(spans.last().unwrap().char_end_in_item, text.chars().count());
        let joined: String = spans
            .iter()
            .map(|s| file.line_text(s.line).unwrap())
            .collect();
        assert_eq!(joined, text, "wrapping must be lossless");
    }

    #[test]
    fn crlf_is_normalized_to_lf() {
        let (data, sid) = seeded(&[user_text("one\r\ntwo")]);
        let reader = data.reader();
        let rows = reader.searchable_rows_blocking(Some(&sid)).unwrap();
        let file = load_transcript_file(&sid, &rows, reader.data_root()).unwrap();
        assert_eq!(file.lines[1], "one");
        assert_eq!(file.lines[2], "two");
        assert!(!file.lines.iter().any(|l| l.contains('\r')));
    }
}

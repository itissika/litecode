//! Canonical read-only session transcript projection.
//!
//! One virtual markdown-like file per session, addressed as
//! `.litecode/sessions/<full_session_id>.md`. SQLite remains the only source of
//! truth; this module never writes files.

use std::path::Path;

use crate::session::data::read_bytes;
use crate::tool::output::{BLOB_PREFIX, blob_dir};
use crate::types::{Item, LitecodeError, Result, item_text_preview};

pub const VIRTUAL_SESSION_DIR: &str = ".litecode/sessions";
pub const VIRTUAL_SESSION_PREFIX: &str = ".litecode/sessions/";
pub const VIRTUAL_SESSION_SUFFIX: &str = ".md";
pub const READ_ONLY_MSG: &str =
    "this path is a read-only session transcript projection; use read or grep";
pub const IN_CONTEXT_WINDOW_MSG: &str =
    "requested lines are already in the current session context window";

pub const SEARCHABLE_KINDS: &[&str] = &[
    "item/user",
    "item/assistant",
    "item/tool_call",
    "item/tool_result",
    "compacted",
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
    pub is_blank: bool,
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

    pub fn first_body_line(&self, seq: i64) -> Option<u32> {
        self.line_index
            .iter()
            .find(|s| s.seq == seq && !s.is_header && !s.is_blank)
            .map(|s| s.line)
    }

    /// Map an item-local char offset to a physical body line.
    pub fn line_for_char(&self, seq: i64, char_start: usize) -> Option<u32> {
        let mut last_body: Option<u32> = None;
        for span in &self.line_index {
            if span.seq != seq || span.is_header || span.is_blank {
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
    let json = if let Some(body) = &row.body {
        body.clone()
    } else if let Some(body_ref) = &row.body_ref {
        match load_blob_text(body_ref, data_root) {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(
                    session_id = %row.session_id,
                    seq = row.seq,
                    error = %e,
                    "session transcript skip unread blob"
                );
                return Ok(None);
            }
        }
    } else {
        return Ok(None);
    };
    let text = if let Ok(item) = serde_json::from_str::<Item>(&json) {
        item_text_preview(&item)
    } else if let Ok(body) = serde_json::from_str::<crate::session::model::CompactedBody>(&json) {
        item_text_preview(&body.agent_item())
    } else {
        tracing::warn!(
            session_id = %row.session_id,
            seq = row.seq,
            "session transcript skip bad item json"
        );
        return Ok(None);
    };
    if text.trim().is_empty() {
        Ok(None)
    } else {
        Ok(Some(normalize_newlines(&text)))
    }
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
    for row in rows {
        if row.session_id != session_id {
            continue;
        }
        let Some(plain) = row_plain_text(row, data_root)? else {
            continue;
        };
        push_item(
            &mut lines,
            &mut line_index,
            row.seq,
            &row.kind,
            &row.item_type,
            &plain,
        );
    }
    Ok(TranscriptFile {
        session_id: session_id.to_string(),
        virtual_path,
        lines,
        line_index,
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
    let header = format!("[seq:{seq} {kind} {item_type}]");
    push_line(
        lines,
        index,
        LineSpan {
            line: 0,
            seq,
            kind: kind.to_string(),
            item_type: item_type.to_string(),
            is_header: true,
            is_blank: false,
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
        let mut end = offset + line_chars;
        let has_more = i + 1 < body_lines.len() || plain.ends_with('\n');
        if has_more && end < total_chars {
            end += 1;
        }
        if end < offset {
            end = offset;
        }
        push_line(
            lines,
            index,
            LineSpan {
                line: 0,
                seq,
                kind: kind.to_string(),
                item_type: item_type.to_string(),
                is_header: false,
                is_blank: false,
                char_start_in_item: offset,
                char_end_in_item: end.max(offset),
            },
            (*body).to_string(),
        );
        offset = end;
    }

    push_line(
        lines,
        index,
        LineSpan {
            line: 0,
            seq,
            kind: kind.to_string(),
            item_type: item_type.to_string(),
            is_header: false,
            is_blank: true,
            char_start_in_item: offset,
            char_end_in_item: offset,
        },
        String::new(),
    );
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
        assert!(file.lines[0].starts_with("[seq:0 item/user "));
        assert_eq!(file.lines[1], "alpha");
        assert_eq!(file.lines[2], "beta NEEDLE gamma");
        assert_eq!(file.lines[3], "delta");
        assert_eq!(file.lines[4], "");

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

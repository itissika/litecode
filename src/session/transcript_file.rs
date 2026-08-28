//! Canonical read-only session transcript projection.
//!
//! One virtual markdown-like file per session, addressed as
//! `.litecode/sessions/<full_session_id>.md`. SQLite remains the only source of
//! truth; this module never writes files.

use std::path::Path;

use rusqlite::{Connection, OpenFlags};

use crate::session::store::data_root_from_db_path;
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

const KIND_SQL: &str =
    "'item/user', 'item/assistant', 'item/tool_call', 'item/tool_result', 'compacted'";

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

/// Canonical virtual paths for every session row. Missing DB → empty.
pub fn list_virtual_paths(db: &Path) -> Result<Vec<String>> {
    if !db.is_file() {
        return Ok(Vec::new());
    }
    let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| LitecodeError::Config(format!("open sessions.db read-only: {e}")))?;
    let mut stmt = conn
        .prepare("SELECT id FROM sessions ORDER BY id ASC")
        .map_err(|e| LitecodeError::Config(format!("list sessions prepare: {e}")))?;
    let ids = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| LitecodeError::Config(format!("list sessions query: {e}")))?;
    let mut out = Vec::new();
    for id in ids {
        let id = id.map_err(|e| LitecodeError::Config(format!("list sessions row: {e}")))?;
        if !id.is_empty() {
            out.push(virtual_path_for(&id));
        }
    }
    Ok(out)
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
    let blob_path = blob_dir(data_root).join(format!("{id}.txt"));
    std::fs::read_to_string(blob_path).map_err(Into::into)
}

pub fn iter_searchable_texts(db_path: &Path) -> Result<Vec<(String, i64, String, String)>> {
    if !db_path.is_file() {
        return Ok(Vec::new());
    }
    let conn = open_ro(db_path)?;
    let data_root = data_root_from_db_path(&db_path.display().to_string());
    let mut stmt = conn
        .prepare(&format!(
            "SELECT t.session_id, t.seq, t.item_type, t.body, t.body_ref
             FROM transcript_items t
             WHERE t.kind IN ({KIND_SQL})
             ORDER BY t.session_id ASC, t.seq ASC"
        ))
        .map_err(|e| LitecodeError::Config(format!("session index prepare: {e}")))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(SearchableRow {
                session_id: row.get(0)?,
                seq: row.get(1)?,
                kind: String::new(),
                item_type: row.get(2)?,
                body: row.get(3)?,
                body_ref: row.get(4)?,
            })
        })
        .map_err(|e| LitecodeError::Config(format!("session index query: {e}")))?;

    let mut out = Vec::new();
    for row in rows {
        let row = row.map_err(|e| LitecodeError::Config(format!("session index row: {e}")))?;
        let Some(text) = row_plain_text(&row, &data_root)? else {
            continue;
        };
        out.push((row.session_id, row.seq, row.item_type, text));
    }
    Ok(out)
}

pub fn load_transcript_file(db_path: &Path, session_id: &str) -> Result<TranscriptFile> {
    let virtual_path = virtual_path_for(session_id);
    if !db_path.is_file() {
        return Ok(TranscriptFile {
            session_id: session_id.to_string(),
            virtual_path,
            lines: Vec::new(),
            line_index: Vec::new(),
        });
    }
    let conn = open_ro(db_path)?;
    let data_root = data_root_from_db_path(&db_path.display().to_string());
    let mut stmt = conn
        .prepare(&format!(
            "SELECT seq, kind, item_type, body, body_ref FROM transcript_items
             WHERE session_id = ?1
               AND kind IN ({KIND_SQL})
             ORDER BY seq ASC"
        ))
        .map_err(|e| LitecodeError::Config(format!("transcript file prepare: {e}")))?;
    let rows = stmt
        .query_map(rusqlite::params![session_id], |row| {
            Ok(SearchableRow {
                session_id: session_id.to_string(),
                seq: row.get(0)?,
                kind: row.get(1)?,
                item_type: row.get(2)?,
                body: row.get(3)?,
                body_ref: row.get(4)?,
            })
        })
        .map_err(|e| LitecodeError::Config(format!("transcript file query: {e}")))?;

    let mut lines = Vec::new();
    let mut line_index = Vec::new();
    for row in rows {
        let row = row.map_err(|e| LitecodeError::Config(format!("transcript file row: {e}")))?;
        let Some(plain) = row_plain_text(&row, &data_root)? else {
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

fn open_ro(db_path: &Path) -> Result<Connection> {
    Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| LitecodeError::Config(format!("open sessions.db read-only: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::store::Session;
    use crate::types::user_text;
    use tempfile::TempDir;

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
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("sessions.db");
        let s = Session::open(db.to_str().unwrap(), "/proj", "default", None).unwrap();
        s.insert_detail_rows(&[user_text("alpha\nbeta NEEDLE gamma\ndelta")])
            .unwrap();
        let sid = s.id.clone();
        drop(s);

        let file = load_transcript_file(&db, &sid).unwrap();
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
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("sessions.db");
        let s = Session::open(db.to_str().unwrap(), "/proj", "default", None).unwrap();
        s.insert_detail_rows(&[user_text("   "), user_text("kept UNIQUE")])
            .unwrap();
        let sid = s.id.clone();
        drop(s);
        let file = load_transcript_file(&db, &sid).unwrap();
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
        let s = Session::open(db.to_str().unwrap(), "/proj", "default", None).unwrap();
        s.insert_detail_rows(&[user_text("old"), user_text("live")])
            .unwrap();
        s.apply_compact_checkpoint_from(&user_text("summary compact body"), Some(1), 3)
            .unwrap();
        let sid = s.id.clone();
        drop(s);
        let file = load_transcript_file(&db, &sid).unwrap();
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
        let s = Session::open(db.to_str().unwrap(), "/proj", "default", None).unwrap();
        let data_root = data_root_from_db_path(db.to_str().unwrap());
        let blobs = blob_dir(&data_root);
        std::fs::create_dir_all(&blobs).unwrap();
        std::fs::write(blobs.join("abc.txt"), r#"{"type":"message","role":"user","content":[{"type":"input_text","text":"BLOB_NEEDLE here"}]}"#).unwrap();
        // Direct SQL insert of a blob-backed user item is heavy; instead verify
        // load_blob_text + row_plain_text which the projection uses.
        let row = SearchableRow {
            session_id: s.id.clone(),
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
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("sessions.db");
        let s = Session::open(db.to_str().unwrap(), "/proj", "default", None).unwrap();
        s.insert_detail_rows(&[user_text("one\r\ntwo")]).unwrap();
        let sid = s.id.clone();
        drop(s);
        let file = load_transcript_file(&db, &sid).unwrap();
        assert_eq!(file.lines[1], "one");
        assert_eq!(file.lines[2], "two");
        assert!(!file.lines.iter().any(|l| l.contains('\r')));
    }
}

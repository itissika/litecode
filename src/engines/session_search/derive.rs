//! The single derivation entry point.
//!
//! Full rebuild and incremental update both call exactly this. Neither may
//! assemble `row_plain_text + chunk_text` on its own: two copies of that
//! pipeline is precisely how a rebuild and an update drift apart, and a derived
//! index that depends on *how* it was reached is worse than a slow one.
//!
//! Pipeline, in order and with no database side effects:
//!
//! ```text
//! SearchableRow
//!   → decode      JSON/blob → projected text + call linkage
//!   → admit       compacted and session-echo results drop out
//!   → hard cut    448-token grid, no overlap, no semantic guessing
//!   → DerivedRow  + normalize, which happens at insert time
//! ```
//!
//! The row's body is never summarised, trimmed or cleaned: the only projection
//! is what `row_plain_text` already defines.

use std::path::Path;

use anyhow::Result;
use tokenizers::Tokenizer;

use crate::session::transcript_file::{SearchableRow, row_plain_text_strict};

use super::chunk::{self, Chunk, ChunkCfg};
use super::echo;

/// Everything the derivation needs that is not the row itself.
pub struct DeriveCfg<'a> {
    pub tk: &'a Tokenizer,
    pub chunk: &'a ChunkCfg,
}

/// What one source row derives to. Carries the facts the index has to remember
/// even for rows it does not index, so a later edit to a related row can be
/// resolved locally instead of by rescanning the corpus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedRow {
    pub session_id: String,
    pub seq: i64,
    pub kind: String,
    pub item_type: String,
    /// The tool call this row belongs to, when it names one. Kept for rows that
    /// are *not* indexed: a rewritten call has to be able to find its result.
    pub call_id: Option<String>,
    /// This row is itself a call into the session store.
    pub session_read_call: bool,
    /// Whether the row contributes chunks at all.
    pub in_chunks: bool,
    /// Chunks of the projected text; empty whenever `in_chunks` is false.
    pub chunks: Vec<Chunk>,
}

/// The `decode` stage on its own: projected text plus call linkage, no admission
/// and no chunking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedRow {
    pub text: Option<String>,
    pub call_id: Option<String>,
    pub session_read_call: bool,
}

/// Decode one row. Fails when the row's source cannot be read — the caller must
/// fail the whole batch rather than skip the row and advance its cursor.
pub fn decode_row(row: &SearchableRow, data_root: &Path) -> Result<DecodedRow> {
    let text = row_plain_text_strict(row, data_root)?
        .map(|raw| raw.trim().to_string())
        .filter(|text| !text.is_empty());
    // A tool call owns a call id and may itself be a session read; a tool result
    // only names the call it answers and is never a "read call" of its own.
    let (call_id, session_read_call) = match echo::call_info(row, data_root) {
        Some(info) => (Some(info.call_id), info.session_read),
        None => (echo::result_call_id(row, data_root), false),
    };
    Ok(DecodedRow {
        text,
        call_id,
        session_read_call,
    })
}

/// Derive one row into its index state.
///
/// `echo_excluded` is this row's admission decision, taken from the shared echo
/// rule; every other decision is made here, once, for both callers.
pub fn derive_row(
    row: &SearchableRow,
    data_root: &Path,
    cfg: &DeriveCfg<'_>,
    echo_excluded: bool,
) -> Result<DerivedRow> {
    let decoded = decode_row(row, data_root)?;
    let admitted = decoded.text.is_some() && row.kind != "compacted" && !echo_excluded;
    let chunks = match (admitted, decoded.text.as_deref()) {
        (true, Some(text)) => chunk::chunk_text(cfg.tk, text, cfg.chunk),
        _ => Vec::new(),
    };
    Ok(DerivedRow {
        session_id: row.session_id.clone(),
        seq: row.seq,
        kind: row.kind.clone(),
        item_type: row.item_type.clone(),
        call_id: decoded.call_id,
        session_read_call: decoded.session_read_call,
        in_chunks: admitted,
        chunks,
    })
}



#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;

    use serde_json::json;

    use crate::session::transcript_file::SearchableRow;

    fn row(kind: &str, item_type: &str, seq: i64, body: serde_json::Value) -> SearchableRow {
        SearchableRow {
            session_id: "s1".into(),
            seq,
            kind: kind.into(),
            item_type: item_type.into(),
            body: Some(body.to_string()),
            body_ref: None,
        }
    }

    fn item_row(kind: &str, seq: i64, item: &crate::types::Item) -> SearchableRow {
        let value = serde_json::to_value(item).expect("serialize item");
        let item_type = value
            .get("type")
            .and_then(|t| t.as_str())
            .unwrap_or("unknown")
            .to_string();
        row(kind, &item_type, seq, value)
    }

    fn user_row(seq: i64, text: &str) -> SearchableRow {
        item_row("item/user", seq, &crate::types::user_text(text))
    }

    fn call_row(seq: i64, call_id: &str, name: &str, args: &str) -> SearchableRow {
        row(
            "item/tool_call",
            "function_call",
            seq,
            json!({
                "type": "function_call",
                "call_id": call_id,
                "name": name,
                "arguments": args,
            }),
        )
    }

    fn result_row(seq: i64, call_id: &str) -> SearchableRow {
        row(
            "item/tool_result",
            "function_call_output",
            seq,
            json!({
                "type": "function_call_output",
                "call_id": call_id,
                "output": "copied page",
            }),
        )
    }

    fn cfg() -> (Tokenizer, ChunkCfg) {
        (
            super::super::tokenizer::open().expect("load tokenizer"),
            super::super::sparse::chunk_cfg(448),
        )
    }

    #[test]
    fn decode_keeps_call_linkage_for_rows_it_does_not_admit() {
        let dir = PathBuf::from(".");
        let call = call_row(1, "c1", "session_search", r#"{"query":"x"}"#);
        let decoded = decode_row(&call, &dir).unwrap();
        assert_eq!(decoded.call_id.as_deref(), Some("c1"));
        assert!(decoded.session_read_call, "a session_search call reads the store");

        let plain = call_row(2, "c2", "read", r#"{"file_path":"src/main.rs"}"#);
        let decoded = decode_row(&plain, &dir).unwrap();
        assert_eq!(decoded.call_id.as_deref(), Some("c2"));
        assert!(!decoded.session_read_call);

        let result = result_row(3, "c1");
        let decoded = decode_row(&result, &dir).unwrap();
        assert_eq!(
            decoded.call_id.as_deref(),
            Some("c1"),
            "an excluded result still records which call it answers"
        );
        assert!(!decoded.session_read_call);
    }

    #[test]
    fn compacted_and_echo_rows_derive_without_chunks() {
        let (tk, chunk) = cfg();
        let cfg = DeriveCfg {
            tk: &tk,
            chunk: &chunk,
        };
        let dir = PathBuf::from(".");

        let compacted = row(
            "compacted",
            "compacted",
            4,
            json!({"summary": "long summary text here", "from": 0, "to": 3}),
        );
        let derived = derive_row(&compacted, &dir, &cfg, false).unwrap();
        assert!(!derived.in_chunks, "compacted never enters chunks");
        assert!(derived.chunks.is_empty());

        let echo = result_row(5, "c1");
        let derived = derive_row(&echo, &dir, &cfg, true).unwrap();
        assert!(!derived.in_chunks, "an echo result is dropped whole");
        assert!(derived.chunks.is_empty());
        assert_eq!(
            derived.call_id.as_deref(),
            Some("c1"),
            "metadata survives the exclusion, so a call rewrite can find it"
        );

        let plain = user_row(6, "hello there");
        let derived = derive_row(&plain, &dir, &cfg, false).unwrap();
        assert!(derived.in_chunks);
        assert!(!derived.chunks.is_empty());
    }
}

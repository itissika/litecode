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
//!   → admit       compacted / echo out; sparse whitelist (三态+tool_call)
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
use super::slots::{self, admits_sparse};

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
    let slot = slots::classify(&row.kind, &row.item_type);
    let admitted =
        decoded.text.is_some() && row.kind != "compacted" && !echo_excluded && admits_sparse(slot);
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
mod tests {}

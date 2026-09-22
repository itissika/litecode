//! Session corpus construction: snapshot rows → slot-projected documents.
//!
//! Admission and projection are handled per policy:
//! * `V0Prod` — the frozen baseline: five searchable kinds, raw text, no trim.
//! * `Final`  — the locked final corpus (see [`slots`]): 人话全留、工具调用留、
//!   工具产出原样（只按预算截断）、压缩总结剔掉。

use anyhow::{Context, Result};
use crate::session::SessionDataReader;
use crate::session::transcript_file::row_plain_text;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokenizers::Tokenizer;

use super::chunk::{self, ChunkCfg};
use super::echo;
use super::slots::{self, Policy, Slot, SlotCfg};

/// One embeddable unit. `key` is the `(session_id, seq)` anchor kept through
/// trimming, so every hit can still resolve back to a physical transcript line.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionDoc {
    pub key: String,
    pub session_id: String,
    pub seq: i64,
    pub kind: String,
    pub item_type: String,
    pub slot: Slot,
    pub tool: Option<String>,
    pub text: String,
    /// Text length of this document before the policy's own transformation.
    pub raw_chars: usize,
    /// `session_id:seq` of the row this document came from. Equal to `key` unless
    /// the row was chunked, in which case several docs share one `row_key` and the
    /// board scores hits per *row*, not per chunk.
    pub row_key: String,
    pub chunk_index: usize,
    pub chunk_total: usize,
    /// Char range this document covers inside the row's projected text. The ranges
    /// of one row tile `[0, chunk_source_chars)` — that tiling is the lossless
    /// invariant the health board verifies.
    pub chunk_start: usize,
    pub chunk_end: usize,
    pub chunk_source_chars: usize,
    /// True for the head+tail projection kept next to a row's faithful chunks.
    /// The lossless tiling is checked over the non-anchor chunks only.
    pub anchor: bool,
}

impl SessionDoc {
    /// True when the budget trim (not chunking) cut the text.
    pub fn trimmed(&self) -> bool {
        self.text.chars().count() < self.raw_chars
    }
}

/// Rows admitted by a policy. `V0Prod` must equal the product corpus exactly:
/// the five searchable kinds, one row = one chunk, raw text.
pub fn build_docs(
    reader: &SessionDataReader,
    policy: Policy,
    cfg: &SlotCfg,
    chunk_cfg: &ChunkCfg,
    tk: Option<&Tokenizer>,
) -> Result<Vec<SessionDoc>> {
    let data_root = reader.data_root();
    let rows = reader
        .searchable_rows_blocking(None)
        .context("read session snapshot rows")?;
    // 会话回声：`session_search` / 指向会话存储的 `read`/`grep` 的产出是复制品；
    // 两条腿一致只留调用行（意图），产出整条剔除（2026-09-22 定）。V0Prod 是
    // 冻结基线，不动。
    let echo_keys = match policy {
        Policy::Final => echo::result_keys(&rows, data_root)?,
        Policy::V0Prod => Default::default(),
    };
    let mut docs = Vec::new();
    let mut last_session = String::new();
    let mut last_tool: Option<String> = None;

    for row in &rows {
        if row.session_id != last_session {
            last_session = row.session_id.clone();
            last_tool = None;
        }
        let slot = slots::classify(&row.kind, &row.item_type);
        // 回声产出：整条不进语料，只留调用行（见 `echo.rs`）。
        if !echo_keys.is_empty()
            && row.kind == "item/tool_result"
            && echo_keys.contains(&(row.session_id.clone(), row.seq))
        {
            continue;
        }
        let kind_ok = matches!(
            row.kind.as_str(),
            "item/user" | "item/assistant" | "item/tool_call" | "item/tool_result" | "compacted"
        );
        let admitted = match policy {
            Policy::V0Prod => kind_ok && slot != Slot::When,
            // 人话 + 工具调用 + 工具产出；压缩总结/时间/未知一律不要。
            Policy::Final => {
                kind_ok
                    && matches!(slot, Slot::Who | Slot::Said | Slot::Thought | Slot::Did | Slot::Outcome)
            }
        };
        // The tool name is needed to label outcome rows, so track it even for
        // rows that are themselves dropped.
        let is_tool_call = slot == Slot::Did;
        let Some(raw) = row_plain_text(row, data_root)? else {
            continue;
        };
        if is_tool_call {
            last_tool = slots::tool_name_of(&raw).map(str::to_string);
        } else if slot != Slot::Who && slot != Slot::Thought && slot != Slot::Said {
            // A new outcome belongs to the action right before it; nothing else
            // should keep a stale label alive.
            if slot == Slot::Summary {
                last_tool = None;
            }
        }
        if !admitted {
            continue;
        }
        let text = match policy {
            Policy::V0Prod => raw.clone(),
            Policy::Final => match slots::row_text_final(slot, &raw, cfg) {
                Some(text) => text,
                None => continue,
            },
        };
        if text.trim().is_empty() {
            continue;
        }
        let row_key = format!("{}:{}", row.session_id, row.seq);
        let tool = if slot == Slot::Outcome { last_tool.clone() } else { None };
        let raw_chars = text.chars().count();

        let chunks = match (policy, tk) {
            (Policy::Final, Some(tk)) if slot.chunks() => chunk::chunk_text(tk, &text, chunk_cfg),
            _ => Vec::new(),
        };
        let chunks_len = chunks.len();
        if chunks.is_empty() {
            let source_chars = text.chars().count();
            docs.push(SessionDoc {
                key: row_key.clone(),
                session_id: row.session_id.clone(),
                seq: row.seq,
                kind: row.kind.clone(),
                item_type: row.item_type.clone(),
                slot,
                tool,
                text,
                raw_chars,
                row_key,
                chunk_index: 0,
                chunk_total: 1,
                chunk_start: 0,
                chunk_end: source_chars,
                chunk_source_chars: source_chars,
                anchor: false,
            });
            continue;
        }
        let source_chars = text.chars().count();
        // Only a *split* row needs an anchor: when the row fits one chunk the
        // chunk text is the head+tail projection verbatim, so an anchor would be
        // a byte-identical duplicate competing with itself.
        if chunk_cfg.anchor && chunks_len > 1 {
            let anchor_text = slots::trim_head_tail(text.trim(), cfg.chars_budget());
            if !anchor_text.trim().is_empty() {
                docs.push(SessionDoc {
                    key: row_key.clone(),
                    session_id: row.session_id.clone(),
                    seq: row.seq,
                    kind: row.kind.clone(),
                    item_type: row.item_type.clone(),
                    slot,
                    tool: tool.clone(),
                    raw_chars: anchor_text.chars().count(),
                    text: anchor_text,
                    row_key: row_key.clone(),
                    chunk_index: 0,
                    chunk_total: 1,
                    chunk_start: 0,
                    chunk_end: source_chars,
                    chunk_source_chars: source_chars,
                    anchor: true,
                });
            }
        }
        let total = chunks.len();
        for c in chunks {
            docs.push(SessionDoc {
                key: if total == 1 {
                    row_key.clone()
                } else {
                    format!("{row_key}#{}", c.index)
                },
                session_id: row.session_id.clone(),
                seq: row.seq,
                kind: row.kind.clone(),
                item_type: row.item_type.clone(),
                slot,
                tool: tool.clone(),
                // For a chunked row `raw_chars` is the chunk's own length: nothing
                // was trimmed, the row was split. `chunk_source_chars` keeps the
                // row-level length for the reachability accounting.
                raw_chars: c.text.chars().count(),
                text: c.text,
                row_key: row_key.clone(),
                chunk_index: c.index,
                chunk_total: total,
                chunk_start: c.start,
                chunk_end: c.end,
                chunk_source_chars: source_chars,
                anchor: false,
            });
        }
    }
    Ok(docs)
}

/// Content hash over the anchor + text, so an index is only reused when the
/// corpus (policy included) is identical.
pub fn content_hash(docs: &[SessionDoc]) -> String {
    let mut hasher = Sha256::new();
    for doc in docs {
        hasher.update(doc.key.as_bytes());
        hasher.update([0x1f]);
        hasher.update(doc.text.as_bytes());
        hasher.update([0x1e]);
    }
    format!("{:x}", hasher.finalize())
}

/// Distinct sessions in corpus order.
pub fn session_ids(docs: &[SessionDoc]) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for doc in docs {
        if seen.insert(doc.session_id.clone()) {
            out.push(doc.session_id.clone());
        }
    }
    out
}

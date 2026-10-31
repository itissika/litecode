//! L2 SemanticEngine — sole algorithmic owner of **Code corpus** semantic retrieval.
//!
//! Production path: embed → Bruch CC (α·dense + (1−α)·BM25) → glob → path dedupe.
//! α = [`retrieve::CODE_SEMANTIC_CC_ALPHA`] (0.8) — **Code workspace only**.
//! Session / Knowledge must not reuse this knob.
//!
//! Unconditional RRF remains available via [`retrieve::bm25_rrf_search`] for
//! eval / ablations; not the product default (hurts semantic main battlefield).

use crate::types::Result;

use super::lexical::{LexicalMatch, LexicalQuery};
use super::lexical_primitive::LexicalPrimitive;
use super::retrieve::{self, CODE_SEMANTIC_CC_ALPHA, SearchHit};
use super::{CodeSearchRuntime, MAX_TOP_K};

/// Production semantic retrieval engine for workspace Code (algorithm lives here only).
#[derive(Debug, Default, Clone, Copy)]
pub struct SemanticEngine;

impl SemanticEngine {
    /// Run Code-corpus semantic search (CC α=0.8). Falls back to ANN-only if
    /// the BM25 sidecar is unavailable.
    pub fn search(
        runtime: &CodeSearchRuntime,
        query: &str,
        glob_filter: Option<&str>,
        top_k: usize,
    ) -> Result<Vec<SearchHit>> {
        let top_k = top_k.clamp(1, MAX_TOP_K);
        let query_vec = runtime.with_embedder(|emb| emb.embed_one(query))?;

        let hits = match runtime.with_index_and_bm25(|index, bm25| {
            retrieve::bm25_cc_search(
                index,
                bm25,
                query,
                &query_vec,
                glob_filter,
                top_k,
                CODE_SEMANTIC_CC_ALPHA,
            )
        }) {
            Ok(hits) => hits,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "code_search CC unavailable; falling back to ann_only"
                );
                runtime.with_index(|index| {
                    retrieve::semantic_search(index, &query_vec, glob_filter, top_k)
                })?
            }
        };
        runtime.note_index_activity();
        Ok(hits)
    }

    /// Prove L1 lexical is borrowable from the semantic engine process.
    pub fn borrow_lexical(query: &LexicalQuery) -> Result<Vec<LexicalMatch>> {
        LexicalPrimitive::search(query)
    }
}

#[cfg(test)]
mod tests {}

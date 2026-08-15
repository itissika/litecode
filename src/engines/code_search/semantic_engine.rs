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
use super::{CodeSearchRuntime, MAX_TOP_K, flush_pending_updates};

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
        flush_pending_updates(runtime);

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
mod tests {
    use super::*;
    use crate::engines::code_search::build::build_full_index;
    use crate::engines::code_search::embed::HashEmbedder;
    use crate::engines::code_search::{CodeSearchRuntime, LexicalQuery};
    use tempfile::TempDir;

    #[test]
    fn search_returns_hits_for_indexed_content() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("main.rs"), "fn auth_login() {}\n").unwrap();

        let mut emb = HashEmbedder;
        let index = build_full_index(root, &mut emb).unwrap();
        index.save(root).unwrap();
        let runtime =
            CodeSearchRuntime::new(root.to_path_buf(), index, Some(Box::new(HashEmbedder)));

        let hits = SemanticEngine::search(&runtime, "auth_login", None, 8).unwrap();
        assert!(!hits.is_empty());
        assert_eq!(hits[0].path, "main.rs");
    }

    #[test]
    fn borrow_lexical_wire_works_from_semantic_engine() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("lib.rs"), "const BORROW_WIRE = 1;\n").unwrap();

        let hits = SemanticEngine::borrow_lexical(&LexicalQuery {
            pattern: "BORROW_WIRE".into(),
            root: root.to_path_buf(),
            path: None,
            case_sensitive: true,
            whole_word: false,
            is_regex: false,
            include: None,
            exclude: None,
            multiline: false,
            max_matches: 5,
            before_context: 0,
            after_context: 0,
            search_hidden: false,
        })
        .unwrap();

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, "lib.rs");
    }
}

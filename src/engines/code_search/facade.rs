//! Retrieval Facade: grouped human search (text ∥ optional semantic).
//!
//! No RRF fusion — columns stay separate for the workspace search UI.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::types::Result;

use super::lexical::LexicalQuery;
use super::lexical_primitive::LexicalPrimitive;
use super::retrieve::SearchHit;
use super::{DEFAULT_TOP_K, MAX_TOP_K};

fn default_corpus() -> String {
    "code".into()
}

fn default_include_semantic() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
pub struct HumanSearchRequest {
    pub query: String,
    /// `code` (default) or `session`.
    #[serde(default = "default_corpus")]
    pub corpus: String,
    #[serde(default)]
    pub case_sensitive: bool,
    #[serde(default)]
    pub whole_word: bool,
    #[serde(default)]
    pub is_regex: bool,
    #[serde(default)]
    pub include: Option<String>,
    #[serde(default)]
    pub exclude: Option<String>,
    #[serde(default)]
    pub top_k: Option<usize>,
    /// Session corpus: 0-based match offset for text pagination.
    #[serde(default)]
    pub offset: Option<usize>,
    /// Session corpus: optional project scope.
    #[serde(default)]
    pub project: Option<String>,
    /// When false, code corpus skips SemanticLane (text-only / fast path).
    /// Session: when false, keeps lexical hits only (skips appended semantic).
    #[serde(default = "default_include_semantic")]
    pub include_semantic: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct HumanSearchResponse {
    pub text: Vec<SearchHit>,
    /// Present only when the semantic engine is Warm and search succeeds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub semantic: Option<Vec<SearchHit>>,
    /// Present when `corpus=session`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_page: Option<crate::engines::session_search::SessionSearchPage>,
}

/// Text (LexicalLane) always runs. `semantic` is attached by the caller when Warm.
pub fn human_search(
    workspace_root: &Path,
    req: &HumanSearchRequest,
    semantic: Option<Vec<SearchHit>>,
) -> Result<HumanSearchResponse> {
    let top_k = req
        .top_k
        .unwrap_or(DEFAULT_TOP_K)
        .clamp(1, MAX_TOP_K.max(100));
    let text_matches = LexicalPrimitive::search(&LexicalQuery {
        pattern: req.query.clone(),
        root: workspace_root.to_path_buf(),
        path: None,
        case_sensitive: req.case_sensitive,
        whole_word: req.whole_word,
        is_regex: req.is_regex,
        include: req.include.clone(),
        exclude: req.exclude.clone(),
        multiline: false,
        max_matches: top_k,
        before_context: 0,
        after_context: 0,
        min_line: 0,
    })?;
    let text: Vec<SearchHit> = text_matches
        .iter()
        .enumerate()
        .map(|(i, m)| m.to_hit(1.0 / (1.0 + i as f64)))
        .collect();

    Ok(HumanSearchResponse {
        text,
        semantic,
        session_page: None,
    })
}

#[cfg(test)]
mod tests {}

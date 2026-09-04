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
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn human_search_text_column_uses_lexical_lane() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.rs"), "alpha\nfind_me_here\nomega\n").unwrap();
        std::fs::write(root.join("b.txt"), "find_me_here\n").unwrap();
        std::fs::write(root.join(".hidden"), "find_me_here\n").unwrap();

        let resp = human_search(
            root,
            &HumanSearchRequest {
                query: "find_me_here".into(),
                corpus: "code".into(),
                case_sensitive: true,
                whole_word: false,
                is_regex: false,
                include: Some("*.rs".into()),
                exclude: None,
                top_k: Some(10),
                offset: None,
                project: None,
                include_semantic: true,
            },
            None,
        )
        .unwrap();

        assert_eq!(resp.text.len(), 1);
        assert!(resp.text[0].path.ends_with("a.rs"));
        assert_eq!(resp.text[0].start_line, 2);
        assert!(resp.text[0].summary.contains("find_me_here"));
        assert!(resp.semantic.is_none());
        assert!(resp.session_page.is_none());
    }

    #[test]
    fn human_search_passes_through_semantic_column_untouched() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("t.txt"), "noop\n").unwrap();
        let sem = vec![SearchHit {
            path: "sem.rs".into(),
            start_line: 3,
            end_line: 5,
            summary: "meaning hit".into(),
            score: 0.9,
        }];
        let resp = human_search(
            root,
            &HumanSearchRequest {
                query: "does_not_exist_zzz".into(),
                corpus: "code".into(),
                case_sensitive: true,
                whole_word: false,
                is_regex: false,
                include: None,
                exclude: None,
                top_k: Some(5),
                offset: None,
                project: None,
                include_semantic: true,
            },
            Some(sem.clone()),
        )
        .unwrap();
        assert!(resp.text.is_empty());
        assert_eq!(resp.semantic.as_ref().unwrap(), &sem);
        assert!(resp.session_page.is_none());
    }

    #[test]
    fn include_semantic_defaults_true_in_json() {
        let req: HumanSearchRequest = serde_json::from_str(r#"{"query":"x"}"#).unwrap();
        assert!(req.include_semantic);
        assert_eq!(req.corpus, "code");
    }

    #[test]
    fn include_semantic_false_parsed() {
        let req: HumanSearchRequest =
            serde_json::from_str(r#"{"query":"x","include_semantic":false}"#).unwrap();
        assert!(!req.include_semantic);
    }
}

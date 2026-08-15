//! L1 LexicalPrimitive — stable in-process text search API.
//!
//! Consumers: agent `grep`, human text column, and (future) SemanticEngine borrow.
//! Implementation is the LexicalLane (`lexical_search`); this type is the only
//! intended call site for new code.

use crate::types::Result;

use super::lexical::{LexicalMatch, LexicalQuery, lexical_search};

/// Borrowable lexical retrieval primitive (no Warm / no index).
#[derive(Debug, Default, Clone, Copy)]
pub struct LexicalPrimitive;

impl LexicalPrimitive {
    pub fn search(query: &LexicalQuery) -> Result<Vec<LexicalMatch>> {
        lexical_search(query)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    #[test]
    fn primitive_searches_workspace_contents() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.rs"), "fn hit_token() {}\n").unwrap();

        let hits = LexicalPrimitive::search(&LexicalQuery {
            pattern: "hit_token".into(),
            root: root.to_path_buf(),
            path: None,
            case_sensitive: true,
            whole_word: false,
            is_regex: false,
            include: None,
            exclude: None,
            multiline: false,
            max_matches: 10,
            before_context: 0,
            after_context: 0,
            search_hidden: false,
        })
        .unwrap();

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, "a.rs");
        let _ = PathBuf::from(&hits[0].path);
    }
}

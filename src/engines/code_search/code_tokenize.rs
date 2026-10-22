//! Code-aware tokenization for sparse retrieval (BM25).
//!
//! Industry practice (coco-search / Retriv): split camelCase / PascalCase /
//! snake_case, keep the original token **and** its parts; no English stemming.

use std::collections::BTreeSet;

/// Expand source/path text into a whitespace-delimited bag for BM25 indexing.
///
/// Example: `getUserById` → `getuserid get user by id` (lowercased).
pub fn expand_for_index(text: &str) -> String {
    let mut out = Vec::new();
    for raw in raw_tokens(text) {
        for part in expand_identifier(&raw) {
            out.push(part);
        }
    }
    out.join(" ")
}

/// Query terms for BM25 (originals + identifier parts), lowercased, deduped.
pub fn query_terms(query: &str) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for raw in raw_tokens(query) {
        for part in expand_identifier(&raw) {
            if part.len() >= 2 && seen.insert(part.clone()) {
                out.push(part);
            }
        }
    }
    out
}

/// Identifier-shaped tokens from a natural-language query (Cody-style signal).
///
/// Keeps camelCase / snake_case / dotted paths; drops short stop-like words.
pub fn extract_identifiers(query: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for raw in raw_tokens(query) {
        if !looks_like_identifier(&raw) {
            continue;
        }
        let lower = raw.to_ascii_lowercase();
        if lower.len() >= 2 && seen.insert(lower.clone()) {
            out.push(lower);
        }
        for part in expand_identifier(&raw) {
            if part.len() >= 2 && seen.insert(part.clone()) {
                out.push(part);
            }
        }
    }
    out
}

fn raw_tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut cur = String::new();
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            cur.push(ch);
        } else if ch == '.' && !cur.is_empty() {
            // Keep dotted paths as separate segments later via split.
            tokens.push(std::mem::take(&mut cur));
        } else if !cur.is_empty() {
            tokens.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        tokens.push(cur);
    }
    tokens
}

fn looks_like_identifier(tok: &str) -> bool {
    if tok.len() < 3 {
        return false;
    }
    let has_upper = tok.chars().any(|c| c.is_ascii_uppercase());
    let has_lower = tok.chars().any(|c| c.is_ascii_lowercase());
    let has_snake = tok.contains('_');
    (has_upper && has_lower) || has_snake || tok.chars().any(|c| c.is_ascii_digit())
}

/// Split one identifier into original (lowercased) plus camel/snake/digit parts.
pub fn expand_identifier(tok: &str) -> Vec<String> {
    if tok.is_empty() {
        return Vec::new();
    }
    let mut parts = Vec::new();
    let lower_orig = tok.to_ascii_lowercase();
    parts.push(lower_orig.clone());

    // snake_case / CONSTANT_CASE
    let snake_parts: Vec<&str> = tok.split('_').filter(|p| !p.is_empty()).collect();
    if snake_parts.len() > 1 {
        for p in snake_parts {
            push_camel_parts(&mut parts, p);
        }
        return dedupe_parts(parts);
    }

    push_camel_parts(&mut parts, tok);
    dedupe_parts(parts)
}

fn push_camel_parts(out: &mut Vec<String>, tok: &str) {
    for piece in split_camel(tok) {
        if piece.len() >= 2 {
            out.push(piece.to_ascii_lowercase());
        } else if piece.len() == 1 && piece.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            out.push(piece.to_ascii_lowercase());
        }
    }
}

/// Split `getUserById` / `HTTPServer2` into word-like pieces (keeps acronyms).
fn split_camel(tok: &str) -> Vec<String> {
    let chars: Vec<char> = tok.chars().collect();
    if chars.is_empty() {
        return Vec::new();
    }
    let mut parts = Vec::new();
    let mut start = 0usize;
    for i in 1..chars.len() {
        let prev = chars[i - 1];
        let cur = chars[i];
        let boundary = (prev.is_ascii_lowercase() && cur.is_ascii_uppercase())
            || (prev.is_ascii_uppercase()
                && cur.is_ascii_uppercase()
                && i + 1 < chars.len()
                && chars[i + 1].is_ascii_lowercase())
            || (prev.is_ascii_alphabetic() && cur.is_ascii_digit())
            || (prev.is_ascii_digit() && cur.is_ascii_alphabetic());
        if boundary {
            parts.push(chars[start..i].iter().collect());
            start = i;
        }
    }
    parts.push(chars[start..].iter().collect());
    parts
}

fn dedupe_parts(parts: Vec<String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for p in parts {
        if p.is_empty() {
            continue;
        }
        if seen.insert(p.clone()) {
            out.push(p);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camel_case_keeps_original_and_parts() {
        let parts = expand_identifier("getUserById");
        assert!(parts.contains(&"getuserbyid".into()));
        assert!(parts.contains(&"get".into()));
        assert!(parts.contains(&"user".into()));
        assert!(parts.contains(&"by".into()));
        assert!(parts.contains(&"id".into()));
    }

    #[test]
    fn snake_case_splits() {
        let parts = expand_identifier("user_repository");
        assert!(parts.contains(&"user_repository".into()));
        assert!(parts.contains(&"user".into()));
        assert!(parts.contains(&"repository".into()));
    }

    #[test]
    fn query_terms_expand_identifiers() {
        let terms = query_terms("where is getUserById defined");
        assert!(terms.iter().any(|t| t == "getuserbyid"));
        assert!(terms.iter().any(|t| t == "user"));
        assert!(terms.iter().any(|t| t == "where"));
    }

    #[test]
    fn extract_identifiers_skips_plain_english() {
        let ids = extract_identifiers("how do we combine vector hits");
        assert!(!ids.iter().any(|t| t == "how"));
        let ids2 = extract_identifiers("call SemanticEngine::search please");
        assert!(ids2.iter().any(|t| t.contains("semantic")));
    }
}

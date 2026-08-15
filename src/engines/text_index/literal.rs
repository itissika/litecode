//! Extract indexable literal runs from search patterns (for trigram queries).

/// Longest literal run suitable for trigram indexing (length >= 3), lowercased.
/// Returns `None` when the pattern cannot be accelerated.
pub fn indexable_literal(pattern: &str, is_regex: bool) -> Option<String> {
    let literal = if is_regex {
        longest_regex_literal_run(pattern)?
    } else {
        pattern.to_string()
    };
    let lower = literal.to_lowercase();
    if lower.chars().count() < 3 {
        return None;
    }
    Some(lower)
}

fn longest_regex_literal_run(pattern: &str) -> Option<String> {
    let mut best = String::new();
    let mut cur = String::new();
    let mut chars = pattern.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            // Escaped char: treat next as literal if present.
            if let Some(n) = chars.next() {
                cur.push(n);
            }
            continue;
        }
        if is_regex_meta(c) {
            if cur.chars().count() > best.chars().count() {
                best = cur.clone();
            }
            cur.clear();
            // Skip simple quantifier after atom already cleared.
            continue;
        }
        cur.push(c);
    }
    if cur.chars().count() > best.chars().count() {
        best = cur;
    }
    if best.chars().count() >= 3 {
        Some(best)
    } else {
        None
    }
}

fn is_regex_meta(c: char) -> bool {
    matches!(
        c,
        '.' | '^' | '$' | '*' | '+' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '|' | '\\'
    )
}

/// Distinct 3-grams from a lowercase literal (max 64 to bound query size).
pub fn trigrams(literal: &str) -> Vec<String> {
    let chars: Vec<char> = literal.chars().collect();
    if chars.len() < 3 {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for i in 0..=chars.len() - 3 {
        if out.len() >= 64 {
            break;
        }
        let g: String = chars[i..i + 3].iter().collect();
        if seen.insert(g.clone()) {
            out.push(g);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_pattern() {
        assert_eq!(
            indexable_literal("HelloWorld", false).as_deref(),
            Some("helloworld")
        );
        assert!(indexable_literal("ab", false).is_none());
    }

    #[test]
    fn regex_extracts_run() {
        let lit = indexable_literal(r"foo.*bar", true).unwrap();
        assert!(lit == "foo" || lit == "bar");
        assert!(indexable_literal(r".*", true).is_none());
        assert_eq!(
            indexable_literal(r"hello_world", true).as_deref(),
            Some("hello_world")
        );
    }

    #[test]
    fn trigram_set() {
        let g = trigrams("abcd");
        assert!(g.contains(&"abc".to_string()));
        assert!(g.contains(&"bcd".to_string()));
    }
}

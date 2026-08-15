//! User-supplied path glob matching (include patterns, agent glob tool).
//!
//! Supports `**`, `*`, `?`, `[abc]`, `{a,b}` brace expansion, comma-separated
//! alternates, and `\` → `/` normalization. Unicode-safe (char-based regex).

use std::sync::Arc;

use regex::Regex;

use crate::types::{LitecodeError, Result};

/// Compiled include/glob matcher for workspace-relative paths (`/` separators).
#[derive(Clone, Debug)]
pub struct PathGlobMatcher {
    re: Arc<Regex>,
}

impl PathGlobMatcher {
    /// Match a workspace-relative path or basename-only patterns like `*.rs`.
    pub fn matches(&self, rel: &str) -> bool {
        let rel = rel.trim_start_matches("./");
        if self.re.is_match(rel) {
            return true;
        }
        rel.rsplit('/')
            .next()
            .filter(|name| !name.is_empty())
            .is_some_and(|name| self.re.is_match(name))
    }
}

/// Normalize user input: backslashes → forward slashes.
pub fn normalize_pattern(pattern: &str) -> String {
    pattern.replace('\\', "/")
}

/// Compile comma- or newline-separated include patterns (e.g. `**/*.ts,**/*.tsx`).
pub fn compile_include_patterns(raw: &str) -> Result<Vec<PathGlobMatcher>> {
    let mut out = Vec::new();
    for part in split_pattern_list(raw) {
        let part = part.strip_prefix('!').unwrap_or(part);
        out.push(compile_include_pattern(part)?);
    }
    Ok(out)
}

/// Split on commas/newlines outside `{…}` / `[…]` groups.
fn split_pattern_list(raw: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut brace_depth = 0i32;
    let mut bracket_depth = 0i32;
    for (i, ch) in raw.char_indices() {
        match ch {
            '{' => brace_depth += 1,
            '}' => brace_depth = brace_depth.saturating_sub(1),
            '[' => bracket_depth += 1,
            ']' => bracket_depth = bracket_depth.saturating_sub(1),
            ',' | '\n' if brace_depth == 0 && bracket_depth == 0 => {
                let part = raw[start..i].trim();
                if !part.is_empty() {
                    parts.push(part);
                }
                start = i + ch.len_utf8();
            }
            _ => {}
        }
    }
    let tail = raw[start..].trim();
    if !tail.is_empty() {
        parts.push(tail);
    }
    parts
}

/// Compile a single user glob pattern.
pub fn compile_include_pattern(pattern: &str) -> Result<PathGlobMatcher> {
    let pattern = normalize_pattern(pattern);
    validate_glob_pattern(&pattern)?;
    let regex_str = glob_to_regex(&pattern);
    let re = Regex::new(&regex_str).map_err(|e| {
        LitecodeError::ToolExecution(format!("invalid glob pattern `{pattern}`: {e}"))
    })?;
    Ok(PathGlobMatcher { re: Arc::new(re) })
}

/// True when `rel` matches any compiled include matcher.
pub fn path_matches_include(rel: &str, matchers: &[PathGlobMatcher]) -> bool {
    matchers.iter().any(|m| m.matches(rel))
}

fn validate_glob_pattern(pattern: &str) -> Result<()> {
    let chars: Vec<char> = pattern.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '[' => {
                let mut j = i + 1;
                while j < chars.len() && chars[j] != ']' {
                    j += 1;
                }
                if j == chars.len() {
                    return Err(LitecodeError::ToolExecution(format!(
                        "invalid glob pattern: unclosed '[' in '{pattern}'"
                    )));
                }
                i = j + 1;
            }
            '{' => {
                let mut depth = 1;
                let mut j = i + 1;
                while j < chars.len() && depth > 0 {
                    match chars[j] {
                        '{' => depth += 1,
                        '}' => depth -= 1,
                        _ => {}
                    }
                    j += 1;
                }
                if depth != 0 {
                    return Err(LitecodeError::ToolExecution(format!(
                        "invalid glob pattern: unclosed '{{' in '{pattern}'"
                    )));
                }
                i = j;
            }
            '\\' => {
                i += 2;
            }
            _ => {
                i += 1;
            }
        }
    }
    Ok(())
}

fn glob_to_regex(pattern: &str) -> String {
    let mut result = String::from("(?s)^");
    let expanded = expand_braces(pattern);
    let chars: Vec<char> = expanded.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '*' => {
                if i + 1 < chars.len() && chars[i + 1] == '*' {
                    if i + 2 < chars.len() && chars[i + 2] == '/' {
                        result.push_str("(?:.*/)?");
                        i += 3;
                    } else if i + 2 < chars.len() && chars[i + 2] == '*' {
                        result.push_str(".*");
                        i += 3;
                    } else {
                        result.push_str(".*");
                        i += 2;
                    }
                } else {
                    result.push_str("[^/]*");
                    i += 1;
                }
            }
            '?' => {
                result.push_str("[^/]");
                i += 1;
            }
            '[' => {
                if let Some(end) = chars[i..].iter().position(|&c| c == ']') {
                    let class_content: String = chars[i + 1..i + end].iter().collect();
                    if class_content.starts_with('!') {
                        result.push_str("[^");
                        result.push_str(&class_content[1..]);
                    } else {
                        result.push('[');
                        result.push_str(&class_content);
                    }
                    result.push(']');
                    i += end + 1;
                } else {
                    result.push_str("\\[");
                    i += 1;
                }
            }
            '.' => {
                result.push_str("\\.");
                i += 1;
            }
            '(' | ')' | '|' => {
                result.push(chars[i]);
                i += 1;
            }
            c => {
                let s = c.to_string();
                if regex::escape(&s) != s {
                    result.push_str(&regex::escape(&s));
                } else {
                    result.push(c);
                }
                i += 1;
            }
        }
    }
    result.push('$');
    result
}

fn expand_braces(pattern: &str) -> String {
    let mut result = String::new();
    let chars: Vec<char> = pattern.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '{' && i + 1 < chars.len() {
            let mut depth = 1;
            let mut j = i + 1;
            while j < chars.len() && depth > 0 {
                match chars[j] {
                    '{' => depth += 1,
                    '}' => depth -= 1,
                    _ => {}
                }
                j += 1;
            }
            if depth == 0 {
                let inner: String = chars[i + 1..j - 1].iter().collect();
                let parts: Vec<&str> = inner.split(',').collect();
                if parts.len() > 1 {
                    result.push('(');
                    for (k, part) in parts.iter().enumerate() {
                        if k > 0 {
                            result.push('|');
                        }
                        result.push_str(&expand_braces(part));
                    }
                    result.push(')');
                    i = j;
                    continue;
                }
            }
        }
        result.push(chars[i]);
        i += 1;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brace_expansion_ts_tsx() {
        let m = compile_include_pattern("**/*.{ts,tsx}").unwrap();
        assert!(m.matches("web/src/foo.ts"));
        assert!(m.matches("web/src/foo.tsx"));
        assert!(!m.matches("web/src/foo.rs"));
    }

    #[test]
    fn backslash_normalized() {
        let m = compile_include_pattern("src\\**\\*.rs").unwrap();
        assert!(m.matches("src/main.rs"));
    }

    #[test]
    fn unicode_path_safe() {
        let m = compile_include_pattern("**/*.rs").unwrap();
        let long = format!("src/中文目录/文件_{}.rs", "x".repeat(4000));
        assert!(m.matches(&long));
    }

    #[test]
    fn comma_separated_patterns() {
        let matchers = compile_include_patterns("**/*.ts,**/*.tsx").unwrap();
        assert!(path_matches_include("a.ts", &matchers));
        assert!(path_matches_include("b.tsx", &matchers));
        assert!(!path_matches_include("c.rs", &matchers));
    }

    #[test]
    fn comma_inside_braces_not_split() {
        let matchers = compile_include_patterns("**/*.{ts,tsx}").unwrap();
        assert_eq!(matchers.len(), 1);
        assert!(path_matches_include("web/src/foo.ts", &matchers));
        assert!(path_matches_include("web/src/foo.tsx", &matchers));
        assert!(!path_matches_include("web/src/foo.rs", &matchers));
    }

    #[test]
    fn basename_only_star() {
        let m = compile_include_pattern("*.rs").unwrap();
        assert!(m.matches("main.rs"));
        assert!(m.matches("src/main.rs"));
    }
}

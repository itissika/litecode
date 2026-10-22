//! Relative-path exclude matching for VS Code-style globs.

use std::collections::HashSet;

use super::path_glob::{PathGlobMatcher, compile_include_pattern, normalize_pattern};
use super::preset::{FilterPreset, exclude_globs};

/// Compiled exclude expression for a preset (or custom glob list).
#[derive(Debug, Clone)]
pub struct ExcludeMatcher {
    /// Path segments that exclude a path when any component equals the name
    /// (e.g. `**/node_modules`, `target`, `**/.git`).
    segments: HashSet<String>,
    /// Patterns that could not be classified as a folder segment — full regex path.
    patterns: Vec<PathGlobMatcher>,
}

impl ExcludeMatcher {
    pub fn from_globs<I, S>(globs: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut segments = HashSet::new();
        let mut patterns = Vec::new();
        for g in globs {
            let g = g.as_ref();
            match classify_exclude_glob(g) {
                ExcludeClass::Segment(name) => {
                    segments.insert(name);
                }
                ExcludeClass::Complex => {
                    if let Ok(m) = compile_include_pattern(g) {
                        patterns.push(m);
                    }
                }
            }
        }
        Self { segments, patterns }
    }

    pub fn for_preset(preset: FilterPreset) -> Self {
        Self::from_globs(exclude_globs(preset))
    }

    pub fn is_empty(&self) -> bool {
        self.segments.is_empty() && self.patterns.is_empty()
    }

    /// True when `rel` (workspace-relative, `/`-separated) is excluded.
    ///
    /// Folder segments fold children (VS Code). Remaining globs match the full
    /// path and ancestors.
    pub fn matches(&self, rel: &str) -> bool {
        if rel.is_empty() || self.is_empty() {
            return false;
        }
        let rel = rel.trim_start_matches("./");
        if !self.segments.is_empty() {
            for component in rel.split('/') {
                if component.is_empty() {
                    continue;
                }
                if self.segments.contains(component) {
                    return true;
                }
            }
        }
        if self.patterns.is_empty() {
            return false;
        }
        if self.patterns.iter().any(|p| path_glob_match(p, rel)) {
            return true;
        }
        for ancestor in path_ancestors(rel) {
            if self.patterns.iter().any(|p| path_glob_match(p, ancestor)) {
                return true;
            }
        }
        false
    }
}

enum ExcludeClass {
    Segment(String),
    Complex,
}

/// Segment basename from a `**/NAME` style exclude glob, if classifiable.
pub fn segment_name_from_exclude_glob(glob: &str) -> Option<String> {
    match classify_exclude_glob(glob) {
        ExcludeClass::Segment(name) => Some(name),
        _ => None,
    }
}

/// Classify product-default-shaped globs for O(components) matching.
///
/// - `**/NAME`, `**/NAME/**`, or bare `NAME` → segment NAME (folder fold)
/// - `*.ext` / `**/*.ext` without other wildcards → Complex (suffix)
/// - anything with `*`, `?`, `[`, `{` in the name part → Complex
fn classify_exclude_glob(glob: &str) -> ExcludeClass {
    let g = normalize_pattern(glob);
    let g = g.trim_start_matches("./");
    let g = g.trim_end_matches('/');
    if g.is_empty() {
        return ExcludeClass::Complex;
    }

    // `**/NAME` or `**/NAME/**`
    if let Some(rest) = g.strip_prefix("**/") {
        let name = rest.strip_suffix("/**").unwrap_or(rest);
        if is_plain_segment(name) {
            return ExcludeClass::Segment(name.to_string());
        }
        return ExcludeClass::Complex;
    }

    // Bare `NAME` / `NAME/` — folder fold (VS Code files.exclude of a directory).
    if is_plain_segment(g) {
        return ExcludeClass::Segment(g.to_string());
    }

    ExcludeClass::Complex
}

fn is_plain_segment(name: &str) -> bool {
    !name.is_empty()
        && !name.contains('/')
        && !name.contains('*')
        && !name.contains('?')
        && !name.contains('[')
        && !name.contains('{')
        && !name.contains('\\')
}

fn path_glob_match(matcher: &PathGlobMatcher, rel: &str) -> bool {
    matcher.matches(rel)
}

fn path_ancestors(rel: &str) -> impl Iterator<Item = &str> {
    let bytes = rel.as_bytes();
    let mut indices = Vec::new();
    for (i, b) in bytes.iter().enumerate() {
        if *b == b'/' && i > 0 {
            indices.push(i);
        }
    }
    indices.into_iter().map(move |i| &rel[..i])
}

/// Convenience: exclude check for a preset without caching the matcher.
pub fn path_excluded(rel: &str, preset: FilterPreset) -> bool {
    ExcludeMatcher::for_preset(preset).matches(rel)
}

#[cfg(test)]
mod tests {
    
    

}

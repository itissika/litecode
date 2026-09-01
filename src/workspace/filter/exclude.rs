//! Relative-path exclude matching for VS Code-style globs.

use std::collections::HashSet;

use super::path_glob::{PathGlobMatcher, compile_include_pattern, normalize_pattern};
use super::preset::{FilterPreset, exclude_globs};

/// Compiled exclude expression for a preset (or custom glob list).
#[derive(Debug, Clone)]
pub struct ExcludeMatcher {
    /// Path segments that exclude a path when any component equals the name
    /// (e.g. `**/node_modules`, `**/.git`).
    segments: HashSet<String>,
    /// Basenames that exclude when the final component matches (e.g. `.DS_Store`).
    /// Overlaps with segments are fine — either hit excludes.
    basenames: HashSet<String>,
    /// Patterns that could not be classified as segment/basename — full regex path.
    patterns: Vec<PathGlobMatcher>,
}

impl ExcludeMatcher {
    pub fn from_globs<I, S>(globs: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut segments = HashSet::new();
        let mut basenames = HashSet::new();
        let mut patterns = Vec::new();
        for g in globs {
            let g = g.as_ref();
            match classify_exclude_glob(g) {
                ExcludeClass::Segment(name) => {
                    segments.insert(name);
                }
                ExcludeClass::Basename(name) => {
                    basenames.insert(name);
                }
                ExcludeClass::Complex => {
                    if let Ok(m) = compile_include_pattern(g) {
                        patterns.push(m);
                    }
                }
            }
        }
        Self {
            segments,
            basenames,
            patterns,
        }
    }

    pub fn for_preset(preset: FilterPreset) -> Self {
        Self::from_globs(exclude_globs(preset))
    }

    pub fn is_empty(&self) -> bool {
        self.segments.is_empty() && self.basenames.is_empty() && self.patterns.is_empty()
    }

    /// True when `rel` (workspace-relative, `/`-separated) is excluded.
    ///
    /// Matches path components against segment/basename sets (VS Code fold children
    /// when a folder segment matches), then remaining regex patterns on the full
    /// path and ancestors.
    pub fn matches(&self, rel: &str) -> bool {
        if rel.is_empty() || self.is_empty() {
            return false;
        }
        let rel = rel.trim_start_matches("./");
        if !self.segments.is_empty() || !self.basenames.is_empty() {
            let mut last = "";
            for component in rel.split('/') {
                if component.is_empty() {
                    continue;
                }
                if self.segments.contains(component) {
                    return true;
                }
                last = component;
            }
            if !last.is_empty() && self.basenames.contains(last) {
                return true;
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
    Basename(String),
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
/// - `**/NAME` or `**/NAME/**` → segment NAME (no wildcards in NAME)
/// - `NAME` or `**/NAME` where NAME looks like a basename file (contains `.` and
///   no path seps beyond the `**/` prefix) still segment if `**/NAME`
/// - `*.ext` / `**/*.ext` without other wildcards → not segment; Complex (suffix)
/// - anything with `*`, `?`, `[`, `{` in the name part → Complex
fn classify_exclude_glob(glob: &str) -> ExcludeClass {
    let g = normalize_pattern(glob);
    let g = g.trim_start_matches("./");

    // `**/NAME` or `**/NAME/**`
    if let Some(rest) = g.strip_prefix("**/") {
        let name = rest.strip_suffix("/**").unwrap_or(rest);
        if is_plain_segment(name) {
            // Basename-only product defaults (`.DS_Store`, `Thumbs.db`) still
            // work as segment checks on any component — same fold semantics.
            return ExcludeClass::Segment(name.to_string());
        }
        return ExcludeClass::Complex;
    }

    // Bare `NAME` with no separators / wildcards → basename
    if is_plain_segment(g) {
        return ExcludeClass::Basename(g.to_string());
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
    use super::*;
    use crate::workspace::filter::defaults::{FILES_EXCLUDE, SEARCH_EXCLUDE, WATCHER_EXCLUDE};

    #[test]
    fn files_exclude_hits_git_and_ds_store() {
        let m = ExcludeMatcher::from_globs(FILES_EXCLUDE);
        assert!(m.matches(".git"));
        assert!(m.matches(".git/config"));
        assert!(m.matches("pkg/.git"));
        assert!(m.matches(".DS_Store"));
        assert!(m.matches("foo/Thumbs.db"));
        assert!(!m.matches("src/main.rs"));
        assert!(!m.matches("node_modules/pkg/index.js"));
    }

    #[test]
    fn text_search_inherits_files_and_search() {
        let m = ExcludeMatcher::for_preset(FilterPreset::TextSearch);
        assert!(m.matches(".git/config"));
        assert!(m.matches("node_modules/x.js"));
        assert!(m.matches("bower_components/y"));
        assert!(m.matches("vendor/bower_components/pkg"));
        assert!(m.matches("foo.code-search"));
        assert!(!m.matches("src/main.rs"));
    }

    #[test]
    fn search_exclude_alone_misses_git() {
        let m = ExcludeMatcher::from_globs(SEARCH_EXCLUDE);
        assert!(!m.matches(".git/config"));
        assert!(m.matches("node_modules/x"));
    }

    #[test]
    fn watcher_excludes_git_objects_not_source() {
        let m = ExcludeMatcher::for_preset(FilterPreset::Watcher);
        assert!(m.matches(".git/objects/ab/cd"));
        assert!(m.matches("nested/.git/objects/ab/cd"));
        assert!(!m.matches("src/main.rs"));
        assert!(!m.matches("node_modules/pkg/index.js"));
    }

    #[test]
    fn watcher_defaults_skip_product_internal_index() {
        let m = ExcludeMatcher::from_globs(WATCHER_EXCLUDE);
        assert!(m.matches(".litecode/index/chunks.jsonl"));
        assert!(m.matches(".litecode/text-index/x"));
        assert!(!m.matches("src/main.rs"));
    }

    #[test]
    fn watcher_excludes_atomic_save_tmp() {
        let m = ExcludeMatcher::for_preset(FilterPreset::Watcher);
        // The atomic-save temp files produced by `WorkspaceService::atomic_write`
        // must be ignored by the watcher so they don't emit spurious events.
        assert!(m.matches("src/.main.rs.litecode-tmp-1-2"));
        assert!(m.matches("nested/dir/.foo.txt.litecode-tmp-99-7"));
        assert!(!m.matches("src/main.rs"));
    }

    #[test]
    fn deep_node_modules_and_bower_segments() {
        let m = ExcludeMatcher::for_preset(FilterPreset::TextSearch);
        assert!(m.matches("a/b/node_modules/c/d.js"));
        assert!(m.matches("vendor/bower_components/pkg/index.js"));
        assert!(m.matches("apps/web/node_modules/.bin/cli"));
    }

    #[test]
    fn classify_segment_and_complex() {
        assert!(matches!(
            classify_exclude_glob("**/node_modules"),
            ExcludeClass::Segment(s) if s == "node_modules"
        ));
        assert!(matches!(
            classify_exclude_glob("**/.git"),
            ExcludeClass::Segment(s) if s == ".git"
        ));
        assert!(matches!(
            classify_exclude_glob("**/*.code-search"),
            ExcludeClass::Complex
        ));
        assert!(matches!(
            classify_exclude_glob("*.litecode-tmp*"),
            ExcludeClass::Complex
        ));
    }
}

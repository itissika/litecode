//! LexicalLane: in-process workspace text search (ripgrep-as-library).
//!
//! Stack ships with the binary via Cargo: `ignore` (walk) + `grep` /
//! `grep-searcher` / `grep-regex` (BurntSushi libripgrep). No PATH `rg`, no
//! subprocess. Human text column and agent `grep` share this module.

use std::path::{Path, PathBuf};

use glob::Pattern;
use grep::regex::RegexMatcherBuilder;
use grep::searcher::{
    BinaryDetection, Searcher, SearcherBuilder, Sink, SinkContext, SinkContextKind, SinkMatch,
};
use ignore::WalkBuilder;

use crate::types::{LitecodeError, Result};
use crate::workspace::filter::{
    FilterPreset, PathGlobMatcher, RelPathCtx, WalkOptions, compile_include_patterns,
    configure_walk_with, path_matches_include,
};

use super::retrieve::SearchHit;

#[derive(Debug, Clone)]
pub struct LexicalQuery {
    pub pattern: String,
    pub root: PathBuf,
    /// Search subdirectory or file under root (optional).
    pub path: Option<PathBuf>,
    pub case_sensitive: bool,
    pub whole_word: bool,
    /// When false, treat pattern as literal (regex-escaped).
    pub is_regex: bool,
    pub include: Option<String>,
    pub exclude: Option<String>,
    pub multiline: bool,
    pub max_matches: usize,
    pub before_context: usize,
    pub after_context: usize,
    /// When true, include hidden files (agent grep default).
    pub search_hidden: bool,
}

#[derive(Debug, Clone)]
pub struct LexicalMatch {
    pub path: String,
    pub start_line: u32,
    pub end_line: u32,
    pub line_text: String,
    pub context_before: Vec<String>,
    pub context_after: Vec<String>,
}

impl LexicalMatch {
    pub fn to_hit(&self, score: f64) -> SearchHit {
        SearchHit {
            path: self.path.clone(),
            start_line: self.start_line,
            end_line: self.end_line,
            summary: self.line_text.trim_end().to_string(),
            score,
        }
    }
}

/// Outcome of a lexical search (matches + scope stats for agent feedback).
#[derive(Debug, Clone)]
pub struct LexicalSearchOutcome {
    pub matches: Vec<LexicalMatch>,
    /// Files that passed include/exclude filters and were searched.
    pub files_searched: usize,
}

/// Run LexicalLane search entirely in-process.
pub fn lexical_search(query: &LexicalQuery) -> Result<Vec<LexicalMatch>> {
    Ok(lexical_search_with_preset(query, FilterPreset::TextSearch)?.matches)
}

/// Run LexicalLane with a consumer-specific workspace filter preset.
///
/// Human workspace search uses [`FilterPreset::TextSearch`]; agent `grep`
/// defaults to [`FilterPreset::AgentText`]. [`FilterPreset::Unfiltered`] is
/// only for explicit `no_ignore` discovery.
pub fn lexical_search_with_preset(
    query: &LexicalQuery,
    preset: FilterPreset,
) -> Result<LexicalSearchOutcome> {
    if query.pattern.is_empty() || query.max_matches == 0 {
        return Ok(LexicalSearchOutcome {
            matches: Vec::new(),
            files_searched: 0,
        });
    }

    // Adaptive TextIndex accelerator (falls back to libripgrep).
    if let Some(accelerated) = crate::engines::text_index::try_accelerated_search(query, preset) {
        return accelerated;
    }

    lexical_search_ripgrep(query, preset)
}

fn lexical_search_ripgrep(
    query: &LexicalQuery,
    preset: FilterPreset,
) -> Result<LexicalSearchOutcome> {
    let search_root = query
        .path
        .as_ref()
        .map(|p| {
            if p.is_absolute() {
                p.clone()
            } else {
                query.root.join(p)
            }
        })
        .unwrap_or_else(|| query.root.clone());

    let pattern = if query.is_regex {
        query.pattern.clone()
    } else {
        regex::escape(&query.pattern)
    };

    let matcher = RegexMatcherBuilder::new()
        .case_insensitive(!query.case_sensitive)
        .word(query.whole_word)
        .multi_line(query.multiline)
        .dot_matches_new_line(query.multiline)
        .build(&pattern)
        .map_err(|e| LitecodeError::ToolExecution(format!("invalid search pattern: {e}")))?;

    let mut searcher = SearcherBuilder::new()
        .line_number(true)
        .multi_line(query.multiline)
        .before_context(query.before_context)
        .after_context(query.after_context)
        // Same default as ripgrep CLI: quit a file when a NUL is observed.
        .binary_detection(BinaryDetection::quit(b'\0'))
        .build();

    let include = compile_include_globs(query.include.as_deref())?;
    let exclude = compile_exclude_globs(query.exclude.as_deref())?;
    let rel_ctx =
        RelPathCtx::new(&query.root).unwrap_or_else(|_| RelPathCtx::new_lossy(&query.root));

    let mut matches = Vec::new();
    let mut files_searched = 0usize;

    if search_root.is_file() {
        let searched = search_one_file(
            &mut searcher,
            &matcher,
            &rel_ctx,
            &search_root,
            &include,
            &exclude,
            /* include_via_walk */ false,
            query.max_matches,
            &mut matches,
        )?;
        return Ok(LexicalSearchOutcome {
            matches,
            files_searched: usize::from(searched),
        });
    }

    let walk_opts = if include.is_empty() {
        WalkOptions::default()
    } else {
        WalkOptions::with_file_include(include.clone())
    };
    let search_ctx =
        RelPathCtx::new(&search_root).unwrap_or_else(|_| RelPathCtx::new_lossy(&search_root));
    let mut walker = WalkBuilder::new(search_ctx.root_lap());
    // Exclude/include matching uses workspace-relative paths (query.root).
    configure_walk_with(&mut walker, rel_ctx.root_lap(), preset, walk_opts);
    // Retained for the human TextSearch caller's explicit hidden-file option.
    // AgentText / FileGlob / Unfiltered own hidden behavior via the preset.
    if preset == FilterPreset::TextSearch {
        walker.hidden(!query.search_hidden);
    }
    walker.parents(true);

    let include_via_walk = !include.is_empty();
    for entry in walker.build() {
        if matches.len() >= query.max_matches {
            break;
        }
        let Ok(entry) = entry else {
            continue;
        };
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let path = entry.path();
        if search_one_file(
            &mut searcher,
            &matcher,
            &rel_ctx,
            path,
            &include,
            &exclude,
            include_via_walk,
            query.max_matches,
            &mut matches,
        )? {
            files_searched += 1;
        }
    }

    Ok(LexicalSearchOutcome {
        matches,
        files_searched,
    })
}

fn search_one_file(
    searcher: &mut Searcher,
    matcher: &grep::regex::RegexMatcher,
    rel_ctx: &RelPathCtx,
    path: &Path,
    include: &[PathGlobMatcher],
    exclude: &[Pattern],
    include_via_walk: bool,
    max_matches: usize,
    out: &mut Vec<LexicalMatch>,
) -> Result<bool> {
    if out.len() >= max_matches {
        return Ok(false);
    }
    let Some(rel) = rel_ctx.rel(path) else {
        return Ok(false);
    };
    if !path_allowed(&rel, include, exclude, include_via_walk) {
        return Ok(false);
    }
    // Binary skip: walk `skip_binary` layer and/or Searcher BinaryDetection::quit.

    let mut sink = MatchSink {
        path: rel,
        out,
        max_matches,
        pending_before: Vec::new(),
        current: None,
    };
    searcher
        .search_path(matcher, path, &mut sink)
        .map_err(|e| LitecodeError::ToolExecution(format!("search {}: {e}", path.display())))?;
    // `finish` already flushes; keep an explicit flush for safety if search
    // short-circuits without calling finish.
    sink.flush_current();
    Ok(true)
}

fn path_allowed(
    rel: &str,
    include: &[PathGlobMatcher],
    exclude: &[Pattern],
    include_via_walk: bool,
) -> bool {
    if !exclude.is_empty() && exclude.iter().any(|p| path_glob_match_exclude(p, rel)) {
        return false;
    }
    if include_via_walk || include.is_empty() {
        return true;
    }
    path_matches_include(rel, include)
}

fn path_glob_match_exclude(pat: &Pattern, rel: &str) -> bool {
    if pat.matches(rel) {
        return true;
    }
    Path::new(rel)
        .file_name()
        .and_then(|f| f.to_str())
        .is_some_and(|name| pat.matches(name))
}

fn compile_include_globs(raw: Option<&str>) -> Result<Vec<PathGlobMatcher>> {
    let Some(raw) = raw.filter(|s| !s.is_empty()) else {
        return Ok(Vec::new());
    };
    compile_include_patterns(raw)
}

fn compile_exclude_globs(raw: Option<&str>) -> Result<Vec<Pattern>> {
    let Some(raw) = raw.filter(|s| !s.is_empty()) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for g in raw
        .split([',', '\n'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let g = g.strip_prefix('!').unwrap_or(g);
        out.push(
            Pattern::new(g)
                .map_err(|e| LitecodeError::ToolExecution(format!("invalid glob `{g}`: {e}")))?,
        );
    }
    Ok(out)
}

struct MatchSink<'a> {
    path: String,
    out: &'a mut Vec<LexicalMatch>,
    max_matches: usize,
    pending_before: Vec<String>,
    current: Option<LexicalMatch>,
}

impl MatchSink<'_> {
    /// Emit the in-flight match (if any). Does not touch `pending_before`:
    /// before-context for the *next* match arrives before `matched` and must
    /// survive a flush of the previous hit.
    fn flush_current(&mut self) {
        if let Some(m) = self.current.take() {
            self.out.push(m);
        }
    }
}

impl Sink for MatchSink<'_> {
    type Error = std::io::Error;

    fn matched(
        &mut self,
        _searcher: &Searcher,
        mat: &SinkMatch<'_>,
    ) -> std::result::Result<bool, Self::Error> {
        if self.out.len() >= self.max_matches {
            return Ok(false);
        }
        self.flush_current();
        if self.out.len() >= self.max_matches {
            return Ok(false);
        }

        let start_line = mat.line_number().unwrap_or(1) as u32;
        let raw = String::from_utf8_lossy(mat.bytes());
        let line_text = raw.trim_end_matches(['\r', '\n']).to_string();
        let extra_lines = line_text.bytes().filter(|&b| b == b'\n').count() as u32;
        let end_line = start_line + extra_lines;

        self.current = Some(LexicalMatch {
            path: self.path.clone(),
            start_line,
            end_line,
            line_text,
            context_before: std::mem::take(&mut self.pending_before),
            context_after: Vec::new(),
        });
        // Keep searching while we still have room for this in-flight hit.
        Ok(self.out.len() < self.max_matches)
    }

    fn context(
        &mut self,
        _searcher: &Searcher,
        context: &SinkContext<'_>,
    ) -> std::result::Result<bool, Self::Error> {
        let text = String::from_utf8_lossy(context.bytes())
            .trim_end_matches(['\r', '\n'])
            .to_string();
        match *context.kind() {
            SinkContextKind::Before => {
                self.pending_before.push(text);
            }
            SinkContextKind::After => {
                if let Some(cur) = self.current.as_mut() {
                    cur.context_after.push(text);
                }
            }
            SinkContextKind::Other => {}
        }
        Ok(true)
    }

    fn context_break(&mut self, _searcher: &Searcher) -> std::result::Result<bool, Self::Error> {
        self.flush_current();
        self.pending_before.clear();
        Ok(self.out.len() < self.max_matches)
    }

    fn finish(
        &mut self,
        _searcher: &Searcher,
        _: &grep::searcher::SinkFinish,
    ) -> std::result::Result<(), Self::Error> {
        self.flush_current();
        self.pending_before.clear();
        Ok(())
    }
}

/// Map language `type` names (grep tool) to include globs.
pub fn type_to_include_globs(t: &str) -> Option<String> {
    let exts: &[&str] = match t {
        "rust" | "rs" => &["*.rs"],
        "python" | "py" => &["*.py"],
        "javascript" | "js" => &["*.js", "*.jsx"],
        "typescript" | "ts" => &["*.ts", "*.tsx"],
        "go" => &["*.go"],
        "java" => &["*.java"],
        "c" => &["*.c", "*.h"],
        "cpp" | "c++" => &["*.cpp", "*.cc", "*.cxx", "*.hpp", "*.hh", "*.hxx"],
        "json" => &["*.json"],
        "toml" => &["*.toml"],
        "yaml" | "yml" => &["*.yaml", "*.yml"],
        "markdown" | "md" => &["*.md"],
        "shell" | "sh" => &["*.sh", "*.bash"],
        "sql" => &["*.sql"],
        _ => return None,
    };
    Some(exts.join(","))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn q(root: &Path, pattern: &str) -> LexicalQuery {
        LexicalQuery {
            pattern: pattern.into(),
            root: root.to_path_buf(),
            path: None,
            case_sensitive: true,
            whole_word: false,
            is_regex: false,
            include: None,
            exclude: None,
            multiline: false,
            max_matches: 50,
            before_context: 0,
            after_context: 0,
            search_hidden: false,
        }
    }

    #[test]
    fn literal_search_finds_line() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.rs"), "fn hello_world() {}\nfn other() {}\n").unwrap();
        let mut query = q(root, "hello_world");
        query.include = Some("*.rs".into());
        let hits = lexical_search(&query).unwrap();
        assert!(!hits.is_empty());
        assert!(hits[0].path.ends_with("a.rs"));
        assert!(hits[0].line_text.contains("hello_world"));
        assert_eq!(hits[0].start_line, 1);
    }

    #[test]
    fn case_insensitive_toggle() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("b.txt"), "HelloWorld\n").unwrap();
        let mut query = q(root, "helloworld");
        assert!(lexical_search(&query).unwrap().is_empty());
        query.case_sensitive = false;
        assert!(!lexical_search(&query).unwrap().is_empty());
    }

    #[test]
    fn whole_word_filter() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        // Substring-only: must not match when whole_word is on.
        std::fs::write(root.join("sub.txt"), "category\n").unwrap();
        let mut query = q(root, "cat");
        query.whole_word = true;
        assert!(
            lexical_search(&query).unwrap().is_empty(),
            "whole_word must reject substring-only hits"
        );

        std::fs::write(root.join("word.txt"), "cat category\n").unwrap();
        let hits = lexical_search(&query).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].path.ends_with("word.txt"));
    }

    #[test]
    fn literal_escapes_regex_metacharacters() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("dot.txt"), "fooX\nfoo.\n").unwrap();

        let mut lit = q(root, "foo.");
        lit.is_regex = false;
        let hits = lexical_search(&lit).unwrap();
        assert_eq!(hits.len(), 1, "literal foo. must not treat '.' as any-char");
        assert!(hits[0].line_text.contains("foo."));

        let mut re = q(root, "foo.");
        re.is_regex = true;
        let hits = lexical_search(&re).unwrap();
        assert_eq!(hits.len(), 2, "regex foo. should match fooX and foo.");
    }

    #[test]
    fn include_brace_pattern() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("web/src")).unwrap();
        std::fs::write(root.join("web/src/a.ts"), "needle\n").unwrap();
        std::fs::write(root.join("web/src/b.tsx"), "needle\n").unwrap();
        std::fs::write(root.join("web/src/c.rs"), "needle\n").unwrap();

        let mut query = q(root, "needle");
        query.include = Some("**/*.{ts,tsx}".into());
        let outcome = lexical_search_with_preset(&query, FilterPreset::Unfiltered).unwrap();
        assert_eq!(outcome.files_searched, 2);
        assert_eq!(outcome.matches.len(), 2);
        assert!(
            outcome
                .matches
                .iter()
                .all(|h| { h.path.ends_with(".ts") || h.path.ends_with(".tsx") })
        );
    }

    #[test]
    fn include_and_exclude_globs() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("keep.rs"), "needle\n").unwrap();
        std::fs::write(root.join("skip.txt"), "needle\n").unwrap();
        std::fs::write(root.join("also.rs"), "needle\n").unwrap();

        let mut query = q(root, "needle");
        query.include = Some("*.rs".into());
        let hits = lexical_search(&query).unwrap();
        assert!(hits.iter().all(|h| h.path.ends_with(".rs")));
        assert_eq!(hits.len(), 2);

        query.exclude = Some("also.rs".into());
        let hits = lexical_search(&query).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].path.ends_with("keep.rs"));
    }

    #[test]
    fn multiline_cross_line() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join("m.rs"),
            "fn foo() {\n    // start\n    let x = 1;\n}\n",
        )
        .unwrap();
        let mut query = q(root, "start\n    let");
        query.is_regex = true;
        query.multiline = true;
        let hits = lexical_search(&query).unwrap();
        assert!(!hits.is_empty());
        assert!(hits[0].line_text.contains("start"));
        assert!(hits[0].line_text.contains("let x = 1") || hits[0].end_line > hits[0].start_line);
    }

    #[test]
    fn context_lines_attached() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join("c.rs"),
            "fn foo() {\n    let x = 1;\n    println!(\"hello\");\n    let y = 2;\n}\n",
        )
        .unwrap();
        let mut query = q(root, "println");
        query.before_context = 1;
        query.after_context = 1;
        let hits = lexical_search(&query).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(
            hits[0]
                .context_before
                .iter()
                .any(|l| l.contains("let x = 1"))
        );
        assert!(
            hits[0]
                .context_after
                .iter()
                .any(|l| l.contains("let y = 2"))
        );
    }

    #[test]
    fn text_search_respects_gitignore() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join(".gitignore"), "ignored.txt\n").unwrap();
        // ignore crate needs a git repo or will still read .gitignore with git_exclude;
        // WalkBuilder.git_ignore reads .gitignore when a .git dir exists OR with
        // standard ignore file discovery. Create .git so gitignore applies.
        std::fs::create_dir(root.join(".git")).unwrap();
        std::fs::write(root.join("ignored.txt"), "secret_needle\n").unwrap();
        std::fs::write(root.join("visible.txt"), "secret_needle\n").unwrap();
        let hits = lexical_search(&q(root, "secret_needle")).unwrap();
        assert!(hits.iter().any(|h| h.path.contains("visible.txt")));
        assert!(!hits.iter().any(|h| h.path.contains("ignored.txt")));
    }

    #[test]
    fn text_search_hidden_files_gated() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join(".secret"), "hidden_token\n").unwrap();
        std::fs::write(root.join("open.txt"), "hidden_token\n").unwrap();

        let mut query = q(root, "hidden_token");
        query.search_hidden = false;
        let hits = lexical_search(&query).unwrap();
        assert!(hits.iter().all(|h| !h.path.contains(".secret")));

        query.search_hidden = true;
        let hits = lexical_search(&query).unwrap();
        assert!(hits.iter().any(|h| h.path.contains(".secret")));
    }

    #[test]
    fn unfiltered_preset_includes_editor_excluded_ignored_and_hidden_files() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("node_modules/pkg")).unwrap();
        std::fs::create_dir(root.join(".git")).unwrap();
        std::fs::write(root.join(".gitignore"), "ignored.txt\n").unwrap();
        std::fs::write(root.join("node_modules/pkg/index.js"), "agent_needle\n").unwrap();
        std::fs::write(root.join("ignored.txt"), "agent_needle\n").unwrap();
        std::fs::write(root.join(".env"), "agent_needle\n").unwrap();

        let outcome =
            lexical_search_with_preset(&q(root, "agent_needle"), FilterPreset::Unfiltered).unwrap();
        let hits = &outcome.matches;
        assert!(
            hits.iter().any(|h| h.path == "node_modules/pkg/index.js"),
            "Unfiltered must not apply search.exclude: {hits:?}"
        );
        assert!(
            hits.iter().any(|h| h.path == "ignored.txt"),
            "Unfiltered must not apply .gitignore: {hits:?}"
        );
        assert!(
            hits.iter().any(|h| h.path == ".env"),
            "Unfiltered must not hide dotfiles: {hits:?}"
        );
    }

    #[test]
    fn agent_text_respects_excludes_keeps_hidden() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("node_modules/pkg")).unwrap();
        std::fs::create_dir(root.join(".git")).unwrap();
        std::fs::write(root.join(".gitignore"), "ignored.txt\n").unwrap();
        std::fs::write(root.join("node_modules/pkg/index.js"), "agent_needle\n").unwrap();
        std::fs::write(root.join("ignored.txt"), "agent_needle\n").unwrap();
        std::fs::write(root.join(".env"), "agent_needle\n").unwrap();
        std::fs::write(root.join("open.txt"), "agent_needle\n").unwrap();

        let outcome =
            lexical_search_with_preset(&q(root, "agent_needle"), FilterPreset::AgentText).unwrap();
        let hits = &outcome.matches;
        assert!(hits.iter().any(|h| h.path == "open.txt"), "{hits:?}");
        assert!(hits.iter().any(|h| h.path == ".env"), "{hits:?}");
        assert!(
            !hits.iter().any(|h| h.path == "ignored.txt"),
            "AgentText must respect .gitignore: {hits:?}"
        );
        assert!(
            !hits.iter().any(|h| h.path.contains("node_modules")),
            "AgentText must apply search.exclude: {hits:?}"
        );
    }

    #[test]
    fn max_matches_is_global() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "x\nx\nx\n").unwrap();
        std::fs::write(root.join("b.txt"), "x\nx\nx\n").unwrap();
        let mut query = q(root, "x");
        query.max_matches = 4;
        let hits = lexical_search(&query).unwrap();
        assert_eq!(hits.len(), 4);
    }

    #[test]
    fn no_path_rg_dependency() {
        // LexicalLane must work with an empty PATH (no external rg).
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("t.txt"), "path_free\n").unwrap();
        let prev = std::env::var_os("PATH");
        unsafe {
            std::env::set_var("PATH", "");
        }
        let result = lexical_search(&q(dir.path(), "path_free"));
        match prev {
            Some(p) => unsafe { std::env::set_var("PATH", p) },
            None => unsafe { std::env::remove_var("PATH") },
        }
        let hits = result.expect("in-process search must not need PATH rg");
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn skips_binary_files_with_nul() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        // Same ASCII needle exists in both; binary must not surface.
        std::fs::write(root.join("ok.txt"), "binary_needle_token\n").unwrap();
        std::fs::write(
            root.join("blob.bin"),
            b"prefix\0binary_needle_token\x01suffix",
        )
        .unwrap();
        // Text extension but contains NUL — still binary by sniff.
        std::fs::write(
            root.join("fake.rs"),
            b"fn x() { /* binary_needle_token */ }\0",
        )
        .unwrap();

        let hits = lexical_search(&q(root, "binary_needle_token")).unwrap();
        assert_eq!(hits.len(), 1, "got: {hits:?}");
        assert!(hits[0].path.ends_with("ok.txt"), "got: {hits:?}");
        assert!(!hits.iter().any(|h| h.path.contains("blob.bin")));
        assert!(!hits.iter().any(|h| h.path.contains("fake.rs")));
    }
}

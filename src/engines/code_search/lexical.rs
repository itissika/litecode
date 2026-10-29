//! LexicalLane: in-process workspace text search (ripgrep-as-library).
//!
//! Stack ships with the binary via Cargo: `ignore` (walk) + `grep` /
//! `grep-searcher` / `grep-regex` (BurntSushi libripgrep). No PATH `rg`, no
//! subprocess. Human text column and agent `grep` share this module.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use glob::Pattern;
use grep::regex::RegexMatcherBuilder;
use grep::searcher::{
    BinaryDetection, Searcher, SearcherBuilder, Sink, SinkContext, SinkContextKind, SinkMatch,
};
use ignore::WalkBuilder;
use tokio_util::sync::CancellationToken;

use crate::types::{LitecodeError, Result};
use crate::workspace::filter::{
    FilterPreset, PathGlobMatcher, RelPathCtx, WalkOptions, cheap_rel_under,
    compile_include_patterns, configure_walk_with, path_matches_include, private_knowledge_walker,
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
    /// Drop matches on this line and earlier. Zero keeps every match.
    /// Used so a later page can continue without storing the prefix.
    pub min_line: u32,
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
            dense_similarity: 0.0,
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
    Ok(lexical_search_with_preset(query, FilterPreset::Search)?.matches)
}

/// Run LexicalLane with a consumer-specific workspace filter preset.
///
/// Human workspace search and agent `grep` share [`FilterPreset::Search`].
/// [`FilterPreset::NoIgnore`] is the agent `-u` hatch.
///
/// This is always a disk walk (libripgrep). The mention path catalog is a
/// separate engine and is not on this path.
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

    lexical_search_ripgrep(query, preset, false, None, None)
}

/// Agent `grep`. Reaches `knowledge/` and `.litecode/knowledge`.
/// The code index stays on [`lexical_search_with_preset`].
pub fn lexical_search_for_agent(
    query: &LexicalQuery,
    preset: FilterPreset,
    workspace: &Path,
) -> Result<LexicalSearchOutcome> {
    lexical_search_for_agent_cancellable(query, preset, workspace, None)
}

/// Agent `grep` with an optional cooperative cancellation token.
///
/// The token is checked between walked files. A single very large file can
/// still outlive the check, but directory-scale searches stop promptly.
pub fn lexical_search_for_agent_cancellable(
    query: &LexicalQuery,
    preset: FilterPreset,
    workspace: &Path,
    cancel: Option<&CancellationToken>,
) -> Result<LexicalSearchOutcome> {
    if query.pattern.is_empty() || query.max_matches == 0 {
        return Ok(LexicalSearchOutcome {
            matches: Vec::new(),
            files_searched: 0,
        });
    }
    lexical_search_ripgrep(query, preset, true, Some(workspace), cancel)
}

fn lexical_search_ripgrep(
    query: &LexicalQuery,
    preset: FilterPreset,
    allow_knowledge_root: bool,
    workspace: Option<&Path>,
    cancel: Option<&CancellationToken>,
) -> Result<LexicalSearchOutcome> {
    if cancel.is_some_and(CancellationToken::is_cancelled) {
        return Err(LitecodeError::ToolExecution("search cancelled".into()));
    }
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
            query.min_line,
            &mut matches,
        );
        return Ok(LexicalSearchOutcome {
            matches,
            files_searched: usize::from(searched),
        });
    }

    let mut walk_opts = if include.is_empty() {
        WalkOptions::default()
    } else {
        WalkOptions::with_file_include(include.clone())
    };
    walk_opts.allow_knowledge_root = allow_knowledge_root;
    let search_ctx =
        RelPathCtx::new(&search_root).unwrap_or_else(|_| RelPathCtx::new_lossy(&search_root));
    let mut walker = WalkBuilder::new(search_ctx.root_lap());
    // Exclude/include matching uses workspace-relative paths (query.root).
    configure_walk_with(&mut walker, rel_ctx.root_lap(), preset, walk_opts.clone());
    let knowledge = workspace.map(|root| root.join(".litecode").join("knowledge"));
    let inside_private = knowledge
        .as_ref()
        .is_some_and(|dir| cheap_rel_under(dir, &search_root).is_some());
    if allow_knowledge_root && inside_private {
        walker
            .git_ignore(false)
            .git_global(false)
            .git_exclude(false);
    }
    walker.parents(true);

    let include_via_walk = !include.is_empty();
    let mut seen = HashSet::new();
    let mut search_entry = |path: &Path| -> bool {
        if matches.len() >= query.max_matches {
            return false;
        }
        let key = cheap_rel_under(&query.root, path).unwrap_or_else(|| path.display().to_string());
        if !seen.insert(key) {
            return true;
        }
        if search_one_file(
            &mut searcher,
            &matcher,
            &rel_ctx,
            path,
            &include,
            &exclude,
            include_via_walk,
            query.max_matches,
            query.min_line,
            &mut matches,
        ) {
            files_searched += 1;
        }
        matches.len() < query.max_matches
    };
    for entry in walker.build() {
        if cancel.is_some_and(CancellationToken::is_cancelled) {
            return Err(LitecodeError::ToolExecution("search cancelled".into()));
        }
        let Ok(entry) = entry else {
            continue;
        };
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        if !search_entry(entry.path()) {
            break;
        }
    }

    if allow_knowledge_root
        && !inside_private
        && let Some(workspace) = workspace
        && let Some(dir) = knowledge.as_ref()
        && dir.is_dir()
        && cheap_rel_under(&search_root, dir).is_some()
        && let Some(extra) = private_knowledge_walker(workspace, preset, walk_opts.skip_binary)
    {
        for entry in extra.build().flatten() {
            if cancel.is_some_and(CancellationToken::is_cancelled) {
                return Err(LitecodeError::ToolExecution("search cancelled".into()));
            }
            if !entry.file_type().is_some_and(|kind| kind.is_file()) {
                continue;
            }
            if !search_entry(entry.path()) {
                break;
            }
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
    min_line: u32,
    out: &mut Vec<LexicalMatch>,
) -> bool {
    if out.len() >= max_matches {
        return false;
    }
    let Some(rel) = rel_ctx.rel(path) else {
        return false;
    };
    if !path_allowed(&rel, include, exclude, include_via_walk) {
        return false;
    }
    // Binary skip: walk `skip_binary` layer and/or Searcher BinaryDetection::quit.

    let mut sink = MatchSink {
        path: rel,
        out,
        max_matches,
        min_line,
        pending_before: Vec::new(),
        current: None,
    };
    // Same as ripgrep: an unreadable file must not abort the walk. On Windows
    // this process holds an exclusive lock on `.litecode/workspace.lock`.
    if let Err(e) = searcher.search_path(matcher, path, &mut sink) {
        tracing::debug!(
            path = %path.display(),
            error = %e,
            "skip unreadable file during lexical search"
        );
        return false;
    }
    // `finish` already flushes; keep an explicit flush for safety if search
    // short-circuits without calling finish.
    sink.flush_current();
    true
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

pub(crate) fn path_glob_match_exclude(pat: &Pattern, rel: &str) -> bool {
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

pub(crate) fn compile_exclude_globs(raw: Option<&str>) -> Result<Vec<Pattern>> {
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
    min_line: u32,
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
        if start_line <= self.min_line {
            return Ok(self.out.len() < self.max_matches);
        }
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
    
    

}

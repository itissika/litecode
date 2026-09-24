//! Agent `grep` tool — LexicalLane frontend.
//!
//! One response shape is chosen from the match count and the token budget:
//! enclosing code for a few code hits, numbered lines otherwise. A hit list whose
//! lines do not all fit opens with the matching paths ranked by hit count and
//! carries the lines that fit; the matches left over are written under
//! `.litecode/bash/` for `read` to page through. Human workspace Search continues
//! to use LexicalLane via the retrieval facade. Both are disk walks; they do not
//! consult the text index.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::context_pipeline::Context;
use crate::engines::code_search::{
    LexicalMatch, LexicalQuery, enclosing_scopes, format_breadcrumb, lexical_search_with_preset,
    lines_slice, syntax_ancestor_snippet,
};
use crate::tool::Tool;
use crate::tool::trait_::ToolExecutionContext;
use crate::types::{LitecodeError, Result, ToolCallResult};
use crate::workspace::filter::{
    FilterPreset, WORKSPACE_EXCLUDES_REL, cheap_rel_under, empty_discovery_hint,
    ignored_discovery_message, looks_binary, split_glob_include_exclude,
};

/// Fixed context lines around each hit when ancestor expansion is unavailable.
const CONTEXT_LINES: usize = 2;
/// Token budget one response gets. Not a model-facing knob: a result over it
/// carries the lines that fit, and the matches left over go to a file.
const GREP_TOKEN_BUDGET: usize = 2_000;
/// At or below this match count, code hits are answered with their enclosing code.
const NARROW_MATCH_MAX: usize = 10;
/// Number of per-file counts in that ranking.
const WIDE_SUMMARY_FILES: usize = 8;
/// At or above this many matching files the ranking is worth opening the page
/// with: below it the map would be a couple of rows, and the lines answer better.
const MAP_MIN_FILES: usize = 4;
/// Extensions whose hits have an enclosing code node worth showing.
const CODE_EXTENSIONS: &[&str] = &[
    "rs", "ts", "tsx", "js", "jsx", "mjs", "cjs", "py", "go", "java", "kt", "kts", "c", "h", "cc",
    "cpp", "cxx", "hpp", "cs", "rb", "php", "swift", "scala", "sh", "bash", "zsh", "ps1", "lua",
    "ex", "exs", "erl", "hs", "ml", "clj", "vue", "svelte", "dart", "sol", "zig", "proto",
];

struct GrepPage {
    body: String,
    warning: Option<String>,
}

impl GrepPage {
    fn ok(body: impl Into<String>) -> Self {
        Self {
            body: body.into(),
            warning: None,
        }
    }
}

/// The shape one response takes. Chosen from the match count, never requested.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GrepShape {
    /// Hits rendered as the code that encloses them.
    Expanded,
    /// Hits rendered with 卤[`CONTEXT_LINES`] lines and no syntax expansion.
    Context,
    /// Hits rendered as one numbered line each.
    Lines,
}

impl GrepShape {
    fn label(self) -> &'static str {
        match self {
            Self::Expanded => "expanded",
            Self::Context => "context",
            Self::Lines => "lines",
        }
    }
}

#[derive(Debug)]
struct GrepOptions {
    pattern: String,
    mode: PatternMode,
    glob: Option<String>,
    include: Option<String>,
    exclude: Option<String>,
    case_sensitive: bool,
}

/// How the raw `pattern` is read by the search.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PatternMode {
    /// The pattern compiled as written.
    Regex,
    /// The pattern does not compile, but escaping its unescaped `{` does: code
    /// text such as `ModelRequest {`, `println!("{}")` or `${VAR}`.
    BraceText,
    /// Nothing compiled: the whole pattern matches literal text.
    Literal,
}

impl GrepOptions {
    /// Whether the engine runs `pattern` as regex. `Literal` hands the raw
    /// pattern to the engine, which escapes it for `is_regex: false`.
    fn is_regex(&self) -> bool {
        self.mode != PatternMode::Literal
    }

    /// Pattern handed to the engine. `BraceText` hands over the brace-escaped
    /// source; `Literal` stays raw, since the engine escapes it for
    /// `is_regex: false`.
    fn query_pattern(&self) -> String {
        match self.mode {
            PatternMode::Regex | PatternMode::Literal => self.pattern.clone(),
            PatternMode::BraceText => escape_bare_braces(&self.pattern),
        }
    }

    /// Pattern handed to the in-process regex matcher (virtual session lines).
    fn regex_source(&self) -> String {
        match self.mode {
            PatternMode::Regex => self.pattern.clone(),
            PatternMode::BraceText => escape_bare_braces(&self.pattern),
            PatternMode::Literal => regex::escape(&self.pattern),
        }
    }

    /// One clause naming a reading that is not the pattern as written, so a
    /// repaired or literal search is never silent.
    fn note(&self) -> Option<String> {
        match self.mode {
            PatternMode::Regex => None,
            PatternMode::BraceText => Some(format!(
                "pattern '{}' has no valid regex reading; unescaped '{{' matched as literal text",
                self.pattern
            )),
            PatternMode::Literal => Some(format!(
                "pattern '{}' is not valid regex; matched as literal text",
                self.pattern
            )),
        }
    }
}

pub struct GrepTool;

impl Tool for GrepTool {
    fn name(&self) -> &str {
        "grep"
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Regular expression over file contents (ripgrep syntax)."
                },
                "path": {
                    "type": "string",
                    "description": "File or directory to search in. Defaults to the workspace."
                },
                "glob": {
                    "type": "string",
                    "description": "Filter which files to search: one glob, or a comma-separated list. A leading ! excludes. E.g. *.rs, **/*.{ts,tsx}, src/**,!**/tests/**. Relative to `path`; do not repeat `path` in the glob."
                },
                "case_sensitive": {
                    "type": "boolean",
                    "description": "Default true. Set false to ignore case."
                }
            },
            "required": ["pattern"]
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    fn execute(
        &self,
        input: Value,
        execution: ToolExecutionContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>> {
        Box::pin(std::future::ready(
            self.call_for_execution(input, execution),
        ))
    }

    fn call_inner(&self, input: Value) -> ToolCallResult {
        // Unit-test helper only. Production Agent turns enter through execute().
        self.call_for_execution(
            input,
            ToolExecutionContext {
                path_mode: crate::workspace::ToolPathMode::All,
                workspace_root: crate::config::workspace::workspace_root_lap(),
                call_id: String::new(),
                cancel: tokio_util::sync::CancellationToken::new(),
                output_limit: self.max_result_size(),
                session_id: String::new(),
                session: None,
            },
        )
    }

    fn description(&self, _ctx: &Context) -> String {
        "Search file contents with a regular expression, case-sensitive by default. Returns matching lines with line numbers, grouped by file. \
         A few code hits come back as the code enclosing them, which often answers what several read calls would.\n\
         A hit list too wide for one response opens with the files that match most, ranked by hit count, and carries the lines that fit; the \
         matches left over are written to a workspace file that read pages through with start_line and end_line. Narrow path or glob instead \
         of trying to raise a limit.\n\
         Example: {\"pattern\":\"fn main\",\"path\":\"src\"}"
            .into()
    }

    fn validate_input(&self, input: &Value) -> std::result::Result<(), String> {
        parse_grep_options(input)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    fn max_result_size(&self) -> usize {
        // grep enforces its own exact token budget before this outer executor cap.
        usize::MAX
    }
}

fn parse_grep_options(input: &Value) -> Result<GrepOptions> {
    let pattern = crate::tool::require_nonempty_string(input, "pattern")
        .map_err(LitecodeError::ToolExecution)?
        .to_string();
    let case_sensitive = optional_bool(input, "case_sensitive")?.unwrap_or(true);
    let mode = pattern_mode(&pattern, case_sensitive);

    let glob = optional_string(input, "glob")?;
    let (include, exclude) = match glob.as_deref() {
        Some(raw) => split_glob_include_exclude(raw),
        None => (None, None),
    };

    Ok(GrepOptions {
        pattern,
        mode,
        glob,
        include,
        exclude,
        case_sensitive,
    })
}

/// Read the pattern the way the search will use it. A pattern that compiles is
/// regex as written. One that does not is usually code text carrying a bare `{`,
/// so try escaping every unescaped `{` and nothing else first: `needle\d+ {`
/// keeps its regex reading, `ModelRequest {` becomes text. Only when that still
/// does not compile is the whole pattern literal text.
fn pattern_mode(pattern: &str, case_sensitive: bool) -> PatternMode {
    if compile_line_regex(pattern, case_sensitive).is_ok() {
        return PatternMode::Regex;
    }
    let escaped_braces = escape_bare_braces(pattern);
    if escaped_braces != pattern && compile_line_regex(&escaped_braces, case_sensitive).is_ok() {
        return PatternMode::BraceText;
    }
    PatternMode::Literal
}

/// `\{` for every `{` that is not already escaped. Only `{` is touched: the rest
/// of the pattern keeps whatever regex reading it had.
fn escape_bare_braces(pattern: &str) -> String {
    let mut out = String::with_capacity(pattern.len());
    let mut backslashes = 0usize;
    for ch in pattern.chars() {
        if ch == '{' && backslashes % 2 == 0 {
            out.push_str("\\{");
        } else {
            out.push(ch);
        }
        backslashes = if ch == '\\' { backslashes + 1 } else { 0 };
    }
    out
}

fn optional_string(input: &Value, name: &str) -> Result<Option<String>> {
    match input.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if value.trim().is_empty() => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        _ => Err(LitecodeError::ToolExecution(format!(
            "parameter '{name}' must be a string"
        ))),
    }
}

fn optional_bool(input: &Value, name: &str) -> Result<Option<bool>> {
    match input.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(value)) => Ok(Some(*value)),
        _ => Err(LitecodeError::ToolExecution(format!(
            "parameter '{name}' must be a boolean"
        ))),
    }
}

fn compile_line_regex(pattern: &str, case_sensitive: bool) -> Result<regex::Regex> {
    regex::RegexBuilder::new(pattern)
        .case_insensitive(!case_sensitive)
        .build()
        .map_err(|e| LitecodeError::ToolExecution(format!("invalid regular expression: {e}")))
}

impl GrepTool {
    fn call_for_execution(&self, input: Value, execution: ToolExecutionContext) -> ToolCallResult {
        match run_grep_page(&input, &execution) {
            Ok(page) => {
                let result = ToolCallResult::ok(page.body);
                match page.warning {
                    Some(warning) => result.with_warning(warning),
                    None => result,
                }
            }
            Err(e) => ToolCallResult::error(e.to_string()),
        }
    }
}

fn run_grep_page(input: &Value, execution: &ToolExecutionContext) -> Result<GrepPage> {
    let options = parse_grep_options(input)?;
    let mut page = search_page(input, &options, execution)?;
    if let Some(note) = options.note() {
        page.warning = Some(match page.warning {
            Some(degraded) => format!("{note}. {degraded}"),
            None => note,
        });
    }
    Ok(page)
}

/// The page one parsed search produces; `run_grep_page` adds the reading note.
fn search_page(
    input: &Value,
    options: &GrepOptions,
    execution: &ToolExecutionContext,
) -> Result<GrepPage> {
    let workspace_root = &execution.workspace_root;
    let path_mode = execution.path_mode;

    if let Some(raw_path) = input["path"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if crate::session::transcript_file::is_virtual_session_path(raw_path) {
            return grep_virtual_session(raw_path, options, execution);
        }
        if crate::session::transcript_file::is_virtual_session_dir(raw_path) {
            return grep_virtual_session_dir(options, execution);
        }
    }

    // Search root is the turn's workspace, or the `path` arg resolved under the
    // tool's permission mode: All admits absolute outside-workspace paths, Safe
    // rejects them here (and in the SAFE preset's explicit deny rule).
    // A file `path` is scoped via LexicalQuery.path under its parent directory so
    // match paths stay relative and snippet rendering can read sources.
    let resolved = match input["path"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(raw_path) => crate::workspace::resolve_agent(workspace_root, raw_path, path_mode)
            .map_err(|e| LitecodeError::ToolExecution(e.to_string()))?,
        None => crate::config::path::canon_abs_lossy(workspace_root),
    };
    let resolved_display = display_search_path(workspace_root, &resolved);
    let file_scoped = resolved.is_file();
    let (root, file_scope): (PathBuf, Option<PathBuf>) = if file_scoped {
        let parent = resolved
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .ok_or_else(|| {
                LitecodeError::ToolExecution(format!(
                    "path has no parent directory: {}",
                    resolved.display()
                ))
            })?;
        (parent.to_path_buf(), Some(resolved.clone()))
    } else if resolved.is_dir() {
        (resolved.clone(), None)
    } else {
        return Err(LitecodeError::ToolExecution(format!(
            "path does not exist: {}",
            resolved.display()
        )));
    };

    if file_scoped && looks_binary(&resolved) {
        return Ok(GrepPage::ok(format!(
            "path '{resolved_display}' is not searched: binary file."
        )));
    }

    let query = LexicalQuery {
        pattern: options.query_pattern(),
        root: root.clone(),
        path: file_scope,
        case_sensitive: options.case_sensitive,
        whole_word: false,
        is_regex: options.is_regex(),
        include: options.include.clone(),
        exclude: options.exclude.clone(),
        multiline: false,
        max_matches: usize::MAX,
        before_context: 0,
        after_context: 0,
    };

    // A `path` that names an excluded tree is refused rather than silently
    // searched: the model asked for that tree specifically, so it must learn the
    // tree is outside the corpus.
    if !file_scoped
        && input["path"]
            .as_str()
            .map(str::trim)
            .is_some_and(|s| !s.is_empty())
        && let Some(message) = ignored_discovery_message(workspace_root, &resolved)
    {
        return Ok(GrepPage::ok(with_path_excluded_ledger(message)));
    }

    let outcome = lexical_search_with_preset(&query, FilterPreset::Search)?;
    if !outcome.matches.is_empty() {
        let searched = outcome.files_searched;
        let matches = sort_grep_matches_vec(outcome.matches);
        return render_matches(&root, &matches, workspace_root, false, Some(searched));
    }

    // Zero matches under the default corpus: the text may live in a tree the
    // Search preset hides (.gitignore / files_exclude / search_exclude). Lift the
    // filters and disclose it instead of asking the model to re-ask.
    if !file_scoped && !matches!(query.path, Some(_)) {
        let lifted = lexical_search_with_preset(&query, FilterPreset::NoIgnore)?;
        if !lifted.matches.is_empty() {
            let searched = lifted.files_searched;
            let matches = sort_grep_matches_vec(lifted.matches);
            return render_matches(&root, &matches, workspace_root, true, Some(searched));
        }
    }

    if let Some(ref pat) = options.glob
        && outcome.files_searched == 0
    {
        return Ok(GrepPage::ok(format_glob_empty(pat, &resolved_display)));
    }
    if outcome.files_searched > 0 {
        return Ok(GrepPage::ok(format_no_hit(outcome.files_searched)));
    }
    if file_scoped {
        return Ok(GrepPage::ok(format!(
            "No matches found (path '{resolved_display}' was not searched)."
        )));
    }
    Ok(GrepPage::ok(format_corpus_empty()))
}

fn format_no_hit(files_searched: usize) -> String {
    format!("No matches found ({}).", searched_scope(files_searched))
}

/// `searched N files`, with the count in the right number.
fn searched_scope(searched: usize) -> String {
    let plural = if searched == 1 { "" } else { "s" };
    format!("searched {searched} file{plural}")
}

fn format_corpus_empty() -> String {
    format!("No matches found. {}", empty_discovery_hint())
}

fn format_glob_empty(pat: &str, resolved_display: &str) -> String {
    format!(
        "No files matched glob '{pat}' under '{resolved_display}'. Use forward slashes; multi-ext like '**/*.ts,**/*.tsx'; or omit glob. {}",
        empty_discovery_hint()
    )
}

fn with_path_excluded_ledger(message: String) -> String {
    if message.contains("LiteCode runtime directory") {
        return format!("{message} To search it anyway, name a single file inside it.");
    }
    let message = if message.contains(".gitignore") {
        format!("{message} git_ignore is on in {WORKSPACE_EXCLUDES_REL}.")
    } else {
        format!("{message} Exclusion lists are in {WORKSPACE_EXCLUDES_REL}.")
    };
    format!("{message} To search it anyway, name a single file inside it.")
}

fn display_search_path(workspace_root: &Path, resolved: &Path) -> String {
    cheap_rel_under(workspace_root, resolved)
        .map(|rel| rel.replace('\\', "/"))
        .filter(|rel| !rel.is_empty())
        .unwrap_or_else(|| resolved.display().to_string())
}

/// A file under `.litecode/bash/` reserved for the matches one response cannot
/// carry. Reserved before the page is fitted, so the footer can name it.
struct SpillSlot {
    path: PathBuf,
    /// Workspace-relative path with `/` separators, ready for `read`.
    location: String,
}

/// The matches one response could not carry, written where `read` can page
/// through them.
struct SpillFile {
    location: String,
    /// How many matches the file holds.
    remaining: usize,
}

/// Reserve the file the tail of a result goes to.
fn spill_slot(workspace_root: &Path) -> Option<SpillSlot> {
    let dir = workspace_root.join(".litecode").join("bash");
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join(format!("grep_{}.txt", crate::terminal::bash_nonce()));
    let location = cheap_rel_under(workspace_root, &path)
        .map(|rel| rel.replace('\\', "/"))
        .unwrap_or_else(|| path.display().to_string());
    Some(SpillSlot { path, location })
}

/// Write the matches the page could not carry, and only those: the page already
/// put the earlier ones in front of the model. Best effort — without the file the
/// page still answers, and the warning says the rest is missing.
fn write_spill(slot: &SpillSlot, matches: &[LexicalMatch], gated: bool) -> Option<SpillFile> {
    if matches.is_empty() {
        return None;
    }
    let mut body = spill_header(matches.len(), gated);
    body.push_str(&format_compact_body(matches));
    std::fs::write(&slot.path, body).ok()?;
    Some(SpillFile {
        location: slot.location.clone(),
        remaining: matches.len(),
    })
}

fn spill_header(remaining: usize, gated: bool) -> String {
    if gated {
        format!(
            "Remaining {remaining} grep matches not shown inline, including paths the default search excludes.\n\n"
        )
    } else {
        format!("Remaining {remaining} grep matches not shown inline.\n\n")
    }
}

/// Footer of a page that could not carry the whole result: where the rest is, and
/// which path the next, narrower search should take.
fn remainder_footer(
    location: &str,
    files: &[(&str, usize)],
    shown_matches: usize,
    total: usize,
) -> String {
    let remaining = total.saturating_sub(shown_matches);
    format!(
        "\nShowing {shown_matches} of {total} matches inline; the remaining {remaining} are in {location}. Read it with start_line and end_line.{}",
        compass_sentence(files, total)
    )
}

/// The narrowing step a wide page suggests: a file that carries a real share of
/// the hits is worth a search of its own, a flat ranking is not.
fn compass_sentence(files: &[(&str, usize)], total: usize) -> String {
    let Some((path, count)) = files.first() else {
        return String::new();
    };
    if files.len() == 1 {
        return format!(
            "\nAll {total} matches are in {path}: narrow the pattern, or read the file itself."
        );
    }
    if *count >= 4 && *count >= (total / files.len()) * 2 {
        format!(
            "\nHottest file '{path}' holds {count} of the {total} matches: re-run grep with path={path} to read its lines inline."
        )
    } else {
        format!(
            "\nHits are spread thin over {} files: narrow path or glob to the subtree you need.",
            files.len()
        )
    }
}

/// The header every grep page starts with, so a degraded view is never silent.
fn page_header(shape: &str, total: usize, gated: bool, files_searched: Option<usize>) -> String {
    let mut scope = files_searched
        .map(|searched| format!("; {}", searched_scope(searched)))
        .unwrap_or_default();
    if gated {
        scope.push_str(if files_searched.is_some() {
            " including excluded paths"
        } else {
            "; searched including excluded paths"
        });
    }
    format!("Found {total} matches ({shape}{scope}):\n")
}

fn page(body: String, header: String, footer: Option<String>) -> String {
    if body.is_empty() {
        return String::new();
    }
    match footer {
        Some(footer) => format!("{header}{body}{footer}"),
        None => format!("{header}{body}"),
    }
}

fn grep_virtual_session(
    raw_path: &str,
    options: &GrepOptions,
    execution: &ToolExecutionContext,
) -> Result<GrepPage> {
    let stem = crate::session::transcript_file::try_parse_virtual_path(raw_path)
        .ok_or_else(|| LitecodeError::ToolExecution("invalid session transcript path".into()))?;
    let reader = execution.session_reader()?;
    let session_id = crate::engines::session_search::resolve_session_ref(reader, &stem)
        .map_err(|e| LitecodeError::ToolExecution(e.to_string()))?;
    let file = reader
        .transcript_file_blocking(&session_id)
        .map_err(|e| LitecodeError::ToolExecution(e.to_string()))?;
    let re = compile_virtual_grep_regex(options)?;
    let matches = grep_transcript_file(reader, &file, &re, &execution.session_id);
    finish_virtual_grep_matches(matches, &execution.workspace_root)
}

fn grep_virtual_session_dir(
    options: &GrepOptions,
    execution: &ToolExecutionContext,
) -> Result<GrepPage> {
    let reader = execution.session_reader()?;
    let listed = crate::session::transcript_file::list_virtual_paths(
        reader.list_session_ids_blocking().unwrap_or_default(),
    );
    let include = options
        .include
        .as_deref()
        .map(crate::workspace::filter::compile_include_patterns)
        .transpose()
        .map_err(|e| LitecodeError::ToolExecution(e.to_string()))?;
    let exclude = options
        .exclude
        .as_deref()
        .map(crate::workspace::filter::compile_include_patterns)
        .transpose()
        .map_err(|e| LitecodeError::ToolExecution(e.to_string()))?;
    let re = compile_virtual_grep_regex(options)?;
    let mut matches = Vec::new();
    for virtual_path in listed {
        if include.as_ref().is_some_and(|matchers| {
            !crate::workspace::filter::path_matches_include(&virtual_path, matchers)
        }) || exclude.as_ref().is_some_and(|matchers| {
            crate::workspace::filter::path_matches_include(&virtual_path, matchers)
        }) {
            continue;
        }
        let Some(stem) = crate::session::transcript_file::try_parse_virtual_path(&virtual_path)
        else {
            continue;
        };
        let file = match reader.transcript_file_blocking(&stem) {
            Ok(f) => f,
            Err(e) => {
                tracing::warn!(path = %virtual_path, error = %e, "skip unread session transcript");
                continue;
            }
        };
        matches.extend(grep_transcript_file(
            reader,
            &file,
            &re,
            &execution.session_id,
        ));
    }
    finish_virtual_grep_matches(matches, &execution.workspace_root)
}

fn compile_virtual_grep_regex(options: &GrepOptions) -> Result<regex::Regex> {
    compile_line_regex(&options.regex_source(), options.case_sensitive)
}

fn grep_transcript_file(
    reader: &crate::session::SessionDataReader,
    file: &crate::session::transcript_file::TranscriptFile,
    re: &regex::Regex,
    caller_session_id: &str,
) -> Vec<LexicalMatch> {
    let hidden = if !caller_session_id.is_empty() && caller_session_id == file.session_id {
        crate::engines::session_search::load_surface_seqs(reader, &file.session_id)
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let mut matches = Vec::new();
    for (i, line) in file.lines.iter().enumerate() {
        let line_no = (i + 1) as u32;
        if file
            .seq_at(line_no)
            .is_some_and(|seq| hidden.contains(&seq))
        {
            continue;
        }
        if re.is_match(line) {
            matches.push(LexicalMatch {
                path: file.virtual_path.clone(),
                start_line: line_no,
                end_line: line_no,
                line_text: line.clone(),
                context_before: Vec::new(),
                context_after: Vec::new(),
            });
        }
    }
    matches
}

fn finish_virtual_grep_matches(
    matches: Vec<LexicalMatch>,
    workspace_root: &Path,
) -> Result<GrepPage> {
    let mut matches = matches;
    sort_grep_matches(&mut matches);
    if matches.is_empty() {
        return Ok(GrepPage::ok("No matches found"));
    }
    render_matches(workspace_root, &matches, workspace_root, false, None)
}

fn sort_grep_matches(matches: &mut [LexicalMatch]) {
    matches.sort_by(|a, b| {
        crate::workspace::glob_hit_key(&a.path)
            .cmp(&crate::workspace::glob_hit_key(&b.path))
            .then(a.start_line.cmp(&b.start_line))
    });
}

fn sort_grep_matches_vec(mut matches: Vec<LexicalMatch>) -> Vec<LexicalMatch> {
    sort_grep_matches(&mut matches);
    matches
}

fn render_matches(
    root: &Path,
    matches: &[LexicalMatch],
    workspace_root: &Path,
    gated: bool,
    files_searched: Option<usize>,
) -> Result<GrepPage> {
    let total = matches.len();
    let shapes = select_shapes(matches);

    // A shape that holds the whole result is the whole answer: nothing pages.
    for shape in &shapes {
        let header = page_header(shape.label(), total, gated, files_searched);
        if shape_fits(root, matches, *shape, &header) {
            return Ok(GrepPage::ok(page(
                render_shape(root, matches, *shape),
                header,
                None,
            )));
        }
    }

    // Too wide to list whole: the page carries the lines that fit behind the file
    // map, and the matches that did not fit go to a file of their own.
    let ranked = ranked_file_counts(matches);
    let header = page_header(GrepShape::Lines.label(), total, gated, files_searched);
    let prologue = format!("{header}{}", orientation_block(&ranked, total));
    let slot = spill_slot(workspace_root);
    // A bound on the footer: same words, widest numbers.
    let footer_bound = slot
        .as_ref()
        .map(|slot| remainder_footer(&slot.location, &ranked, total, total))
        .unwrap_or_default();
    let (lines, shown) = match fit_shape(root, matches, GrepShape::Lines, &prologue, &footer_bound)
    {
        Some(slice) => (slice.body, slice.shown),
        None => (String::new(), 0),
    };
    if shown == total {
        // The whole result fit after all: no file, no pointer.
        return Ok(GrepPage::ok(page(lines, prologue, None)));
    }
    let spilled = slot.and_then(|slot| write_spill(&slot, &matches[shown..], gated));
    let footer = spilled
        .as_ref()
        .map(|file| remainder_footer(&file.location, &ranked, shown, total));
    let warning = Some(match &spilled {
        Some(file) => format!(
            "Showing {shown} of {total} matches; the other {} are in the file below.",
            file.remaining
        ),
        None => format!(
            "Showing {shown} of {total} matches; the other {} could not be written to a file.",
            total - shown
        ),
    });
    if lines.is_empty() {
        // Not even one line fits: the count and the file map still answer.
        return Ok(GrepPage {
            body: format!("{prologue}{}", footer.unwrap_or_default()),
            warning,
        });
    }
    Ok(GrepPage {
        body: page(lines, prologue, footer),
        warning,
    })
}

/// True when the whole match list renders as `shape` inside one response.
fn shape_fits(root: &Path, matches: &[LexicalMatch], shape: GrepShape, header: &str) -> bool {
    crate::session::count_text_tokens(&format!("{header}{}", render_shape(root, matches, shape)))
        <= GREP_TOKEN_BUDGET
}

/// Body of one shape over the whole match list.
fn render_shape(root: &Path, matches: &[LexicalMatch], shape: GrepShape) -> String {
    match shape {
        GrepShape::Expanded | GrepShape::Context => format_snippet_body(root, matches, shape),
        GrepShape::Lines => format_compact_body(matches),
    }
}

/// Shapes this result may take, richest first. The first candidate is the shape
/// the result *should* take; the rest are the narrower ones it falls back to when
/// the result will not fit one response.
///
/// A handful of code matches is cheap enough to answer with the code enclosing
/// each hit; everything else is answered line by line, whatever the count. Hits
/// spread over enough files to make a map of are answered with the map when the
/// lines do not fit one response, so the next search has somewhere to narrow to.
/// The caller never picks: the shape follows the result size.
fn select_shapes(matches: &[LexicalMatch]) -> Vec<GrepShape> {
    let mut shapes = Vec::new();
    if matches.len() <= NARROW_MATCH_MAX && files_are_code(matches) {
        shapes.push(GrepShape::Expanded);
        shapes.push(GrepShape::Context);
    }
    shapes.push(GrepShape::Lines);
    shapes
}

/// Distinct paths carrying a match, in path order.
fn hit_paths(matches: &[LexicalMatch]) -> Vec<&str> {
    let mut paths: Vec<&str> = matches.iter().map(|m| m.path.as_str()).collect();
    paths.sort_unstable();
    paths.dedup();
    paths
}

/// Majority of hit files carry a source extension. A narrow hit set spread over
/// prose (markdown, logs, transcripts) is answered line by line: there is no
/// enclosing code to show.
fn files_are_code(matches: &[LexicalMatch]) -> bool {
    let files = hit_paths(matches);
    if files.is_empty() {
        return false;
    }
    let code = files
        .iter()
        .filter(|path| {
            Path::new(path)
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| {
                    CODE_EXTENSIONS
                        .iter()
                        .any(|known| known.eq_ignore_ascii_case(ext))
                })
        })
        .count();
    code * 2 >= files.len()
}

/// One shape rendered inside the budget, ready to be embedded in an over-budget
/// page.
struct ShapeSlice {
    body: String,
    /// How many rows of the shape the slice carries.
    shown: usize,
}

/// Cut one shape's body to the largest prefix whose page fits the budget. `None`
/// when not even the first row fits, so the caller falls back rather than
/// emitting an over-budget page.
///
/// `prologue` (header + orientation) and `epilogue` (a bound on the partial-page
/// footer) are counted with the body: the whole page has to fit.
fn fit_shape(
    root: &Path,
    matches: &[LexicalMatch],
    shape: GrepShape,
    prologue: &str,
    epilogue: &str,
) -> Option<ShapeSlice> {
    match shape {
        GrepShape::Expanded | GrepShape::Context => {
            let (body, shown) = fitting_prefix(matches, prologue, epilogue, |page| {
                format_snippet_body(root, page, shape)
            })?;
            Some(ShapeSlice { body, shown })
        }
        GrepShape::Lines => {
            let (body, shown) = fitting_prefix(matches, prologue, epilogue, format_compact_body)?;
            Some(ShapeSlice { body, shown })
        }
    }
}

/// Largest `items[..n]` whose page fits the budget, with the `n` it kept.
/// `None` when not even one item fits, so a caller falls back rather than
/// emitting an over-budget page.
fn fitting_prefix<T: Clone>(
    items: &[T],
    prologue: &str,
    epilogue: &str,
    body_of: impl Fn(&[T]) -> String,
) -> Option<(String, usize)> {
    let fits = |n: usize| {
        let body = body_of(&items[..n]);
        crate::session::count_text_tokens(&format!("{prologue}{body}{epilogue}"))
            <= GREP_TOKEN_BUDGET
    };
    if items.is_empty() || !fits(1) {
        return None;
    }
    let (mut low, mut high) = (1usize, items.len());
    while low < high {
        let mid = low + (high - low).div_ceil(2);
        if fits(mid) {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    Some((body_of(&items[..low]), low))
}

fn format_compact_body(matches: &[LexicalMatch]) -> String {
    let mut body = String::new();
    let mut current: Option<&str> = None;
    for m in matches {
        if current != Some(m.path.as_str()) {
            body.push_str(&m.path);
            body.push('\n');
            current = Some(&m.path);
        }
        let text = crate::tool::snippet::truncate_snippet_lines(
            m.line_text.trim_end_matches(['\n', '\r']),
        );
        body.push_str(&crate::tool::format_file_line(m.start_line, &text));
    }
    body
}

fn ranked_file_counts(matches: &[LexicalMatch]) -> Vec<(&str, usize)> {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for hit in matches {
        *counts.entry(&hit.path).or_default() += 1;
    }
    let mut files: Vec<_> = counts.into_iter().collect();
    files.sort_by(|(left_path, left_count), (right_path, right_count)| {
        right_count.cmp(left_count).then(
            crate::workspace::glob_hit_key(left_path)
                .cmp(&crate::workspace::glob_hit_key(right_path)),
        )
    });
    files
}

/// Per-file ranking that opens a page whose lines did not all fit. The lines below
/// it need a location, and the ranking is what the next, narrower search goes by.
fn orientation_block(files: &[(&str, usize)], total: usize) -> String {
    if files.len() < MAP_MIN_FILES {
        return String::new();
    }
    let mut body = format!("{total} matches in {} files. Most matches:\n", files.len());
    for (path, count) in files.iter().take(WIDE_SUMMARY_FILES) {
        body.push_str(&format!("{count:>6} {path}\n"));
    }
    if files.len() > WIDE_SUMMARY_FILES {
        body.push_str(&format!(
            "        (and {} more files)\n",
            files.len() - WIDE_SUMMARY_FILES
        ));
    }
    body.push('\n');
    body
}

/// Zed grep-panel style: page grouped by file (`## Matches in {path}`) with
/// AST-breadcrumb headings per hit. File order is glob_hit_key (same as compact).
fn format_snippet_body(root: &Path, matches: &[LexicalMatch], view: GrepShape) -> String {
    let mut file_order: Vec<String> = Vec::new();
    let mut by_file: BTreeMap<String, Vec<&LexicalMatch>> = BTreeMap::new();
    for m in matches {
        if !by_file.contains_key(&m.path) {
            file_order.push(m.path.clone());
        }
        by_file.entry(m.path.clone()).or_default().push(m);
    }

    // Per-file source cache for AST breadcrumbs + ancestor snippets.
    let mut file_sources: HashMap<String, Option<String>> = HashMap::new();

    let mut sections = Vec::new();
    for path in file_order {
        let Some(file_matches) = by_file.get(&path) else {
            continue;
        };

        let source = file_sources
            .entry(path.clone())
            .or_insert_with(|| std::fs::read_to_string(root.join(&path)).ok())
            .as_deref();

        let ranges = merge_snippet_ranges(file_matches, source, &path, view == GrepShape::Expanded);
        for range in ranges {
            let breadcrumb = (view == GrepShape::Expanded)
                .then(|| {
                    source.and_then(|src| {
                        format_breadcrumb(&enclosing_scopes(&path, src, range.hit_line))
                    })
                })
                .flatten();
            sections.push(crate::tool::SnippetSection {
                path: path.clone(),
                start_line: range.start_line,
                end_line: range.end_line,
                breadcrumb,
                text: range.text,
                remaining_lines: range.remaining_lines,
            });
        }
    }

    crate::tool::format_snippet_sections(&sections)
}

struct SnippetRange {
    start_line: u32,
    end_line: u32,
    /// Primary match line used for enclosing-scope lookup.
    hit_line: u32,
    text: String,
    remaining_lines: u32,
}

fn merge_snippet_ranges(
    file_matches: &[&LexicalMatch],
    source: Option<&str>,
    path: &str,
    use_ancestor: bool,
) -> Vec<SnippetRange> {
    let mut ranges: Vec<SnippetRange> = Vec::new();
    for m in file_matches {
        let built = snippet_for_match(m, source, path, use_ancestor);
        if let Some(last) = ranges.last_mut()
            && built.start_line <= last.end_line.saturating_add(1)
        {
            if built.end_line > last.end_line {
                last.end_line = built.end_line;
                last.text = if let Some(src) = source {
                    lines_slice(src, last.start_line, last.end_line)
                } else {
                    merge_snippet_text(&last.text, &built.text)
                };
            }
            // Keep the larger remaining hint if either side was truncated.
            last.remaining_lines = last.remaining_lines.max(built.remaining_lines);
            continue;
        }
        ranges.push(built);
    }
    ranges
}

fn snippet_for_match(
    m: &LexicalMatch,
    source: Option<&str>,
    path: &str,
    use_ancestor: bool,
) -> SnippetRange {
    let match_end = m.end_line.max(m.start_line);

    if use_ancestor
        && let Some(src) = source
        && let Some(ancestor) = syntax_ancestor_snippet(path, src, m.start_line, match_end)
    {
        return SnippetRange {
            start_line: ancestor.start_line,
            end_line: ancestor.end_line,
            hit_line: m.start_line,
            text: lines_slice(src, ancestor.start_line, ancestor.end_line),
            remaining_lines: ancestor.remaining_lines,
        };
    }

    // Fallback: 卤CONTEXT_LINES from lexical hit (and optional source rebuild).
    if let Some(src) = source {
        let line_count = src.lines().count() as u32;
        let start = m.start_line.saturating_sub(CONTEXT_LINES as u32).max(1);
        let end = match_end
            .saturating_add(CONTEXT_LINES as u32)
            .min(line_count.max(1));
        return SnippetRange {
            start_line: start,
            end_line: end,
            hit_line: m.start_line,
            text: lines_slice(src, start, end),
            remaining_lines: 0,
        };
    }

    let (start, end, text) = snippet_from_lexical_fields(m);
    SnippetRange {
        start_line: start,
        end_line: end,
        hit_line: m.start_line,
        text,
        remaining_lines: 0,
    }
}

fn snippet_from_lexical_fields(m: &LexicalMatch) -> (u32, u32, String) {
    let mut lines: BTreeMap<u32, String> = BTreeMap::new();
    let first = m.start_line;
    let last = m.end_line.max(m.start_line);

    for (i, line) in m.context_before.iter().enumerate() {
        let line_no = first.saturating_sub((m.context_before.len() - i) as u32);
        if line_no >= 1 {
            lines.insert(line_no, line.clone());
        }
    }

    let match_text = m.line_text.trim_end_matches('\n');
    if match_text.contains('\n') {
        for (i, line) in match_text.split('\n').enumerate() {
            lines.insert(first + i as u32, line.to_string());
        }
    } else {
        lines.insert(first, match_text.to_string());
    }

    for (i, line) in m.context_after.iter().enumerate() {
        lines.insert(last + 1 + i as u32, line.clone());
    }

    if lines.is_empty() {
        return (first, last, String::new());
    }
    let start = *lines.keys().next().unwrap();
    let end = *lines.keys().next_back().unwrap();
    let mut text = String::new();
    for (i, (_ln, line)) in lines.iter().enumerate() {
        if i > 0 {
            text.push('\n');
        }
        text.push_str(line);
    }
    (start, end, text)
}

fn merge_snippet_text(a: &str, b: &str) -> String {
    // Naive union by lines: keep unique lines in order of first appearance.
    let mut out: Vec<&str> = Vec::new();
    for line in a.split('\n').chain(b.split('\n')) {
        if !out.contains(&line) {
            out.push(line);
        }
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static CWD_LOCK: Mutex<()> = Mutex::new(());

    #[allow(dead_code)]
    fn with_cwd<R>(dir: &std::path::Path, f: impl FnOnce() -> R) -> R {
        let _guard = CWD_LOCK.lock().expect("cwd lock");
        let prev = std::env::current_dir().expect("prev cwd");
        std::env::set_current_dir(dir).expect("set cwd");
        let out = f();
        let _ = std::env::set_current_dir(prev);
        out
    }

    fn execution(
        dir: &std::path::Path,
        path_mode: crate::workspace::ToolPathMode,
    ) -> ToolExecutionContext {
        ToolExecutionContext {
            path_mode,
            workspace_root: dir.to_path_buf(),
            call_id: String::new(),
            cancel: tokio_util::sync::CancellationToken::new(),
            output_limit: GrepTool.max_result_size(),
            session_id: String::new(),
            session: Some(crate::session::SessionDataReader::open(
                &dir.join(".litecode").join("sessions.db"),
            )),
        }
    }

    fn call_result_mode(
        dir: &std::path::Path,
        input: Value,
        path_mode: crate::workspace::ToolPathMode,
    ) -> ToolCallResult {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");
        rt.block_on(GrepTool.execute(input, execution(dir, path_mode)))
    }

    fn call_result(dir: &std::path::Path, input: Value) -> ToolCallResult {
        call_result_mode(dir, input, crate::workspace::ToolPathMode::Safe)
    }

    fn call_in(dir: &std::path::Path, input: Value) -> String {
        call_result(dir, input).content
    }

    fn call_in_mode(
        dir: &std::path::Path,
        input: Value,
        path_mode: crate::workspace::ToolPathMode,
    ) -> String {
        call_result_mode(dir, input, path_mode).content
    }

    fn write(dir: &std::path::Path, rel: &str, body: &str) {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, body).unwrap();
    }

    /// Path named by the spill footer, in the wording the tool uses.
    fn spill_path(page: &str) -> Option<String> {
        let (before, _) = page.split_once(". Read it with start_line and end_line.")?;
        let (_, location) = before.rsplit_once(" are in ")?;
        Some(location.trim().trim_end_matches('.').to_string())
    }

    /// Thousands of hits spread over many files: the file ranking itself is too
    /// long for one response, so the complete result can only live in a file.
    fn write_many_files(dir: &std::path::Path, count: usize) {
        for f in 0..count {
            write(
                dir,
                &format!("deeply/nested/directory/number/{f:03}/source_file.rs"),
                "needle\n",
            );
        }
    }

    const NESTED_RS: &str = "fn main() {\n    if ready {\n        needle_here();\n    }\n}\n";

    // -----------------------------------------------------------------
    // 1. Four-parameter surface
    // -----------------------------------------------------------------

    #[test]
    fn schema_exposes_exactly_four_parameters() {
        let props = GrepTool.schema().get("properties").unwrap().clone();
        let keys: Vec<&str> = props
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(keys, vec!["case_sensitive", "glob", "path", "pattern"]);
        assert_eq!(
            GrepTool.schema()["required"],
            serde_json::json!(["pattern"])
        );
        // A stale or guessed parameter must not discard the search: the schema
        // leaves unknown keys to the executor's warning path.
        assert!(
            GrepTool.schema().get("additionalProperties").is_none(),
            "an unknown parameter warns instead of failing the call"
        );
        assert!(
            crate::tool::check_tool_input(
                &GrepTool,
                &serde_json::json!({"pattern": "x", "context": 3}),
            )
            .is_ok(),
            "a guessed parameter must not reject the call"
        );
    }

    #[test]
    fn removed_knobs_do_not_reach_the_model() {
        let schema = GrepTool.schema();
        let props = schema.get("properties").unwrap();
        for gone in [
            "output_mode",
            "offset",
            "token_budget",
            "-i",
            "-w",
            "-u",
            "-F",
            "limit",
            "head_limit",
            "max_results",
        ] {
            assert!(props.get(gone).is_none(), "{gone} must not be advertised");
        }
    }

    #[test]
    fn description_names_the_shapes_and_the_recovery_file() {
        let ctx = crate::context_pipeline::Context {
            cwd: std::path::PathBuf::from("."),
            workspace_paths: crate::config::WorkspacePaths::for_legacy_root(
                &std::path::PathBuf::from("."),
            ),
            agents_md: None,
            claude_md: None,
        };
        let d = GrepTool.description(&ctx);
        assert!(d.contains("case-sensitive by default"), "got: {d}");
        assert!(d.contains("code enclosing them"), "got: {d}");
        assert!(
            d.contains("answers what several read calls would"),
            "got: {d}"
        );
        assert!(d.contains("files that match most"), "got: {d}");
        assert!(d.contains("written to a workspace file"), "got: {d}");
        assert!(d.contains("start_line and end_line"), "got: {d}");
        assert!(!d.contains("output_mode"), "got: {d}");
        assert!(!d.contains("token_budget"), "got: {d}");
    }

    #[test]
    fn pattern_is_required_and_must_be_nonempty() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            GrepTool
                .validate_input(&serde_json::json!({}))
                .unwrap_err()
                .contains("pattern")
        );
        assert!(
            GrepTool
                .validate_input(&serde_json::json!({"pattern": ""}))
                .is_err()
        );
        assert!(
            GrepTool
                .validate_input(&serde_json::json!({"pattern": "x"}))
                .is_ok()
        );
        let _ = dir;
    }

    #[test]
    fn a_bare_brace_matches_code_text_and_says_so() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "a.rs",
            "pub struct ModelRequest {\n    model: String,\n}\n",
        );
        assert!(
            GrepTool
                .validate_input(&serde_json::json!({"pattern": "ModelRequest {"}))
                .is_ok(),
            "a code-shaped pattern must not fail the call"
        );
        let result = call_result(dir.path(), serde_json::json!({"pattern": "ModelRequest {"}));
        assert!(
            result.content.contains("pub struct ModelRequest {"),
            "got: {}",
            result.content
        );
        let warning = result.warning_status.expect("the reading must be named");
        assert!(warning.contains("unescaped '{'"), "got: {warning}");
    }

    #[test]
    fn a_bare_brace_leaves_the_rest_of_the_pattern_as_regex() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.rs", "needle_42 {\n");
        write(dir.path(), "b.rs", "needle_x {\nother_42 {\n");
        let out = call_in(dir.path(), serde_json::json!({"pattern": "needle_\\d+ {"}));
        assert!(out.contains("needle_42 {"), "got: {out}");
        assert!(
            !out.contains("needle_x") && !out.contains("other_42"),
            "the digit class must stay regex, got: {out}"
        );
    }

    #[test]
    fn a_pattern_with_no_regex_reading_becomes_literal_text() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.rs", "let hit = found[0];\n");
        assert!(
            GrepTool
                .validate_input(&serde_json::json!({"pattern": "found["}))
                .is_ok(),
            "an unreadable pattern must not fail the call"
        );
        let result = call_result(dir.path(), serde_json::json!({"pattern": "found["}));
        assert!(
            result.content.contains("found[0]"),
            "got: {}",
            result.content
        );
        let warning = result.warning_status.expect("the reading must be named");
        assert!(warning.contains("literal text"), "got: {warning}");
    }

    #[test]
    fn a_valid_pattern_keeps_its_regex_reading() {
        assert_eq!(pattern_mode("needle", true), PatternMode::Regex);
        assert_eq!(pattern_mode(r"a{2,3}", true), PatternMode::Regex);
        assert_eq!(pattern_mode(r"ModelRequest \{", true), PatternMode::Regex);
        assert_eq!(pattern_mode("ModelRequest {", true), PatternMode::BraceText);
        assert_eq!(pattern_mode(r"needle\d+ {", true), PatternMode::BraceText);
        assert_eq!(pattern_mode("println!(\"{}\")", true), PatternMode::BraceText);
        assert_eq!(pattern_mode("found[", true), PatternMode::Literal);
        assert_eq!(escape_bare_braces("ModelRequest {"), "ModelRequest \\{");
        assert_eq!(escape_bare_braces(r"a{2,3}"), r"a\{2,3}");
        assert_eq!(escape_bare_braces(r"a\{"), r"a\{");
        assert_eq!(escape_bare_braces(r"a\\{"), r"a\\\{");
    }

    #[test]
    fn path_scopes_the_search_to_one_file() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.rs", "needle\n");
        write(dir.path(), "b.rs", "needle\n");
        let out = call_in(
            dir.path(),
            serde_json::json!({"pattern": "needle", "path": "a.rs"}),
        );
        assert!(out.contains("a.rs"), "got: {out}");
        assert!(!out.contains("b.rs"), "got: {out}");
    }

    #[test]
    fn missing_path_reports_the_resolved_path() {
        let dir = tempfile::tempdir().unwrap();
        let out = call_in(
            dir.path(),
            serde_json::json!({"pattern": "x", "path": "nope/missing"}),
        );
        assert!(out.contains("path does not exist"), "got: {out}");
        assert!(out.contains("missing"), "got: {out}");
    }

    #[test]
    fn outside_workspace_path_is_denied_under_safe() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.rs", "needle\n");
        // A second tempdir stands in for any tree outside the workspace: the
        // point is the mode gate, not the size of the target.
        let outside = tempfile::tempdir().unwrap();
        write(outside.path(), "b.rs", "absent_token\n");
        let outside_path = outside.path().join("b.rs").display().to_string();
        let out = call_in(
            dir.path(),
            serde_json::json!({"pattern": "needle", "path": outside_path}),
        );
        assert!(
            out.contains("SAFE mode only permits paths under the workspace"),
            "Safe must refuse an outside path, got: {out}"
        );
        // The same tool admits it once the binding is unrestricted, and answers
        // from the outside file.
        let admitted = call_in_mode(
            dir.path(),
            serde_json::json!({"pattern": "absent_token", "path": outside_path}),
            crate::workspace::ToolPathMode::All,
        );
        assert!(!admitted.contains("SAFE mode"), "got: {admitted}");
        assert!(admitted.contains("b.rs"), "got: {admitted}");
    }

    #[test]
    fn case_sensitive_defaults_true_and_false_ignores_case() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "g.rs", "Hello World\n");
        assert!(
            call_in(dir.path(), serde_json::json!({"pattern": "hello world"}))
                .contains("No matches found")
        );
        let insensitive = call_in(
            dir.path(),
            serde_json::json!({"pattern": "hello world", "case_sensitive": false}),
        );
        assert!(insensitive.contains("Hello World"), "got: {insensitive}");
    }

    #[test]
    fn case_sensitive_must_be_a_boolean() {
        let err = GrepTool
            .validate_input(&serde_json::json!({"pattern": "x", "case_sensitive": "smart"}))
            .unwrap_err();
        assert!(err.contains("boolean"), "{err}");
    }

    #[test]
    fn glob_filters_files_and_brace_alternation_works() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.ts", "needle\n");
        write(dir.path(), "b.tsx", "needle\n");
        write(dir.path(), "c.rs", "needle\n");
        let out = call_in(
            dir.path(),
            serde_json::json!({"pattern": "needle", "glob": "**/*.{ts,tsx}"}),
        );
        assert!(out.contains("a.ts") && out.contains("b.tsx"), "got: {out}");
        assert!(!out.contains("c.rs"), "got: {out}");
    }

    #[test]
    fn glob_bang_prefix_filters_files_out() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "src/keep.rs", "needle\n");
        write(dir.path(), "tests/skip.rs", "needle\n");
        let out = call_in(
            dir.path(),
            serde_json::json!({"pattern": "needle", "glob": "**/*.rs,!**/tests/**"}),
        );
        assert!(out.contains("keep.rs"), "got: {out}");
        assert!(!out.contains("skip.rs"), "got: {out}");
    }

    #[test]
    fn glob_that_matches_nothing_explains_itself() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "only.rs", "needle\n");
        let out = call_in(
            dir.path(),
            serde_json::json!({"pattern": "needle", "glob": "**/*.{ts,tsx}"}),
        );
        assert!(out.contains("No files matched glob"), "got: {out}");
        assert!(out.contains("forward slashes"), "got: {out}");
    }

    #[test]
    fn a_no_hit_names_the_corpus_bounds_instead_of_a_knob() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.rs", "nothing here\n");
        let out = call_in(dir.path(), serde_json::json!({"pattern": "absent_token"}));
        assert!(
            out.starts_with("No matches found (searched 1 file)."),
            "got: {out}"
        );
        assert!(
            !out.contains("-u"),
            "a miss must not name a removed knob, got: {out}"
        );
    }

    // -----------------------------------------------------------------
    // 2. Automatic view
    // -----------------------------------------------------------------

    #[test]
    fn few_code_matches_render_the_enclosing_code() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.rs", NESTED_RS);
        let out = call_in(dir.path(), serde_json::json!({"pattern": "needle_here"}));
        assert!(out.starts_with("Found 1 matches (expanded"), "got: {out}");
        assert!(
            out.contains("; searched 1 file)"),
            "the header names the searched scope, got: {out}"
        );
        assert!(out.contains("## Matches in a.rs"), "got: {out}");
        // The tightest node enclosing the hit is the `if` block, not the function.
        assert!(out.contains("### fn main › L2-4"), "got: {out}");
        assert!(out.contains("needle_here();"), "got: {out}");
        assert!(!out.contains("file below"), "1 match must not spill: {out}");
    }

    #[test]
    fn narrow_code_hits_in_a_huge_function_stay_bounded_snippets() {
        // A hit inside a huge function must not drag the whole node in: the
        // expanded view keeps the snippet tight and answers in one page.
        let dir = tempfile::tempdir().unwrap();
        let filler: String = (0..400).map(|i| format!("    let v{i} = {i};\n")).collect();
        let body = format!("fn huge() {{\n{filler}    needle();\n{filler}}}\n");
        write(dir.path(), "huge.rs", &body);
        let out = call_in(dir.path(), serde_json::json!({"pattern": "needle"}));
        assert!(out.starts_with("Found 1 matches (expanded"), "got: {out}");
        assert!(
            !out.contains("file below"),
            "one bounded snippet fits: {out}"
        );
        assert!(out.contains("needle();"), "got: {out}");
        assert!(
            out.lines().count() < 40,
            "the snippet stays tight, got {} lines",
            out.lines().count()
        );
    }

    #[test]
    fn a_narrow_hit_in_prose_stays_line_oriented() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "notes.md", "alpha\nneedle\nomega\n");
        let out = call_in(dir.path(), serde_json::json!({"pattern": "needle"}));
        assert!(out.starts_with("Found 1 matches (lines"), "got: {out}");
        assert!(
            !out.contains("## Matches in"),
            "prose has no enclosing code, got: {out}"
        );
    }

    #[test]
    fn a_wide_result_within_one_page_stays_line_oriented() {
        let dir = tempfile::tempdir().unwrap();
        let body: String = (0..30).map(|i| format!("needle {i}\n")).collect();
        write(dir.path(), "a.rs", &body);
        let out = call_in(dir.path(), serde_json::json!({"pattern": "needle"}));
        assert!(out.starts_with("Found 30 matches (lines"), "got: {out}");
        assert!(out.contains("    30: needle 29"), "got: {out}");
        assert!(!out.contains("file below"), "30 short lines fit: {out}");
    }

    #[test]
    fn a_wide_result_still_gets_matching_lines() {
        let dir = tempfile::tempdir().unwrap();
        for f in 0..60 {
            write(dir.path(), &format!("f{f:02}.rs"), "needle\n");
        }
        let out = call_in(dir.path(), serde_json::json!({"pattern": "needle"}));
        assert!(out.starts_with("Found 60 matches (lines"), "got: {out}");
        assert_eq!(
            out.matches(": needle").count(),
            60,
            "every match is a numbered line, got: {out}"
        );
        assert!(
            !out.contains("file below"),
            "60 short lines fit one page: {out}"
        );
    }

    #[test]
    fn a_wide_result_too_long_to_list_pages_map_lines_and_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        write_many_files(dir.path(), 300);
        let out = call_in(dir.path(), serde_json::json!({"pattern": "needle"}));
        assert!(out.starts_with("Found 300 matches (lines"), "got: {out}");
        // The page opens with the map: where the hits are, ranked by count.
        assert!(
            out.contains("300 matches in 300 files. Most matches:"),
            "got: {out}"
        );
        assert!(out.contains("(and 292 more files)"), "got: {out}");
        // …then carries real lines, never counts alone.
        let inline = out.matches(": needle").count();
        assert!(
            inline > 0 && inline < 300,
            "the page must carry lines, got {inline}"
        );
        // …and ends with the compass: what the page left out and how to narrow down.
        assert!(
            out.contains(&format!(
                "Showing {inline} of 300 matches inline; the remaining {} are in ",
                300 - inline
            )),
            "got: {out}"
        );
        assert!(
            out.contains("Hits are spread thin over 300 files"),
            "the page must say how to narrow down, got: {out}"
        );
        let rel = spill_path(&out).expect("the rest must be named");
        let spilled = std::fs::read_to_string(dir.path().join(&rel)).unwrap();
        assert_eq!(
            spilled.matches(": needle").count(),
            300 - inline,
            "the file holds the matches the page did not, and only those"
        );
        assert!(
            spilled.starts_with(&format!(
                "Remaining {} grep matches not shown inline",
                300 - inline
            )),
            "got: {}",
            &spilled[..spilled.len().min(80)]
        );
    }

    #[test]
    fn a_wide_page_carries_lines_and_names_the_hottest_file() {
        let dir = tempfile::tempdir().unwrap();
        let body = |count: usize| -> String {
            (0..count)
                .map(|i| format!("item{i:03} {}\n", "x".repeat(20)))
                .collect()
        };
        write(dir.path(), "src/hot.rs", &body(260));
        for f in 0..5 {
            write(dir.path(), &format!("src/m{f}.rs"), &body(20));
        }
        let out = call_in(dir.path(), serde_json::json!({"pattern": "item\\d+"}));
        assert!(out.starts_with("Found 360 matches (lines"), "got: {out}");
        assert!(
            out.contains("360 matches in 6 files. Most matches:"),
            "got: {out}"
        );
        assert!(
            out.contains("   260 src/hot.rs"),
            "the map leads with the hotspot, got: {out}"
        );
        assert!(
            out.contains("Hottest file 'src/hot.rs' holds 260 of the 360 matches: re-run grep with path=src/hot.rs"),
            "the compass must say which path to search next, got: {out}"
        );
        let inline = out.matches(": item").count();
        assert!(inline > 0 && inline < 360, "got {inline}");
        let rel = spill_path(&out).expect("the rest must be named");
        let spilled = std::fs::read_to_string(dir.path().join(&rel)).unwrap();
        assert_eq!(
            spilled.matches(": item").count(),
            360 - inline,
            "only the rest is in the file"
        );
    }

    #[test]
    fn a_wide_result_that_fits_does_not_spill() {
        let dir = tempfile::tempdir().unwrap();
        for f in 0..30 {
            write(dir.path(), &format!("f{f:02}.rs"), "needle\n");
        }
        let out = call_in(dir.path(), serde_json::json!({"pattern": "needle"}));
        assert!(out.starts_with("Found 30 matches (lines"), "got: {out}");
        assert!(!out.contains("file below"), "got: {out}");
    }

    #[test]
    fn shape_selection_keeps_lines_at_any_match_count() {
        let chunk = |count: usize, path: &str| {
            (0..count)
                .map(|i| LexicalMatch {
                    path: path.into(),
                    start_line: (i + 1) as u32,
                    end_line: (i + 1) as u32,
                    line_text: "needle\n".into(),
                    context_before: Vec::new(),
                    context_after: Vec::new(),
                })
                .collect::<Vec<_>>()
        };
        let first = |count: usize, path: &str| select_shapes(&chunk(count, path))[0];
        assert_eq!(first(1, "a.rs"), GrepShape::Expanded);
        assert_eq!(first(10, "a.rs"), GrepShape::Expanded);
        assert_eq!(first(11, "a.rs"), GrepShape::Lines);
        assert_eq!(first(50, "a.rs"), GrepShape::Lines);
        // A wide list is still answered line by line: the over-budget part goes to
        // the complete-result file, never to bare counts.
        assert_eq!(first(51, "a.rs"), GrepShape::Lines);
        assert_eq!(first(5_000, "a.rs"), GrepShape::Lines);
        // Prose never gets enclosing code, however few the hits are.
        assert_eq!(first(3, "notes.md"), GrepShape::Lines);
        // Every shape list ends at the line view: a hit list is answered with lines
        // however many files it spans, and whatever does not fit pages under it.
        assert_eq!(
            select_shapes(&chunk(1, "a.rs")),
            vec![GrepShape::Expanded, GrepShape::Context, GrepShape::Lines]
        );
        assert_eq!(
            select_shapes(&chunk(5_000, "a.rs")).last().copied(),
            Some(GrepShape::Lines)
        );
        let mut spread = chunk(1, "a.rs");
        for path in ["b.rs", "c.rs", "d.rs", "e.rs"] {
            spread.extend(chunk(1, path));
        }
        assert_eq!(
            select_shapes(&spread),
            vec![GrepShape::Expanded, GrepShape::Context, GrepShape::Lines]
        );
    }

    #[test]
    fn files_are_code_needs_a_majority_of_source_files() {
        let one = |path: &str| LexicalMatch {
            path: path.into(),
            start_line: 1,
            end_line: 1,
            line_text: "x".into(),
            context_before: Vec::new(),
            context_after: Vec::new(),
        };
        assert!(files_are_code(&[one("a.rs")]));
        assert!(files_are_code(&[one("a.rs"), one("b.ts")]));
        assert!(!files_are_code(&[one("a.md")]));
        assert!(!files_are_code(&[one("a.rs"), one("b.md"), one("c.md")]));
        assert!(!files_are_code(&[]));
    }

    #[test]
    fn binary_path_is_refused_without_searching() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("blob.bin"), b"\x00\x01\x02needle\n").unwrap();
        let out = call_in(
            dir.path(),
            serde_json::json!({"pattern": "needle", "path": "blob.bin"}),
        );
        assert!(out.contains("binary file"), "got: {out}");
    }

    // -----------------------------------------------------------------
    // 3. Spill over budget
    // -----------------------------------------------------------------

    #[test]
    fn an_over_budget_result_writes_the_rest_to_a_file() {
        let dir = tempfile::tempdir().unwrap();
        write_many_files(dir.path(), 300);
        let result = call_result(dir.path(), serde_json::json!({"pattern": "needle"}));
        let page = &result.content;
        assert!(page.starts_with("Found 300 matches (lines"), "got: {page}");
        let inline = page.matches(": needle").count();
        let rel = spill_path(page).expect("footer must name the rest");
        let spilled = std::fs::read_to_string(dir.path().join(&rel)).unwrap();
        assert!(
            spilled.starts_with(&format!(
                "Remaining {} grep matches not shown inline.",
                300 - inline
            )),
            "got: {}",
            &spilled[..spilled.len().min(80)]
        );
        assert_eq!(
            spilled.matches(": needle").count(),
            300 - inline,
            "the file holds the matches the page did not, and only those"
        );
        assert_eq!(
            spilled.matches("source_file.rs\n").count(),
            300 - inline,
            "one section per file the page left out"
        );
        let warning = result.warning_status.expect("a paged view must warn");
        assert!(
            warning.contains(&format!("Showing {inline} of 300 matches")),
            "got: {warning}"
        );
        assert!(warning.contains("file below"), "got: {warning}");
    }

    #[test]
    fn a_result_that_fits_never_spills() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "small.rs", "needle\n");
        let result = call_result(dir.path(), serde_json::json!({"pattern": "needle"}));
        assert!(
            !result.content.contains("file below"),
            "got: {}",
            result.content
        );
        assert!(
            result.warning_status.is_none(),
            "got: {:?}",
            result.warning_status
        );
        assert!(
            !dir.path().join(".litecode/bash").exists(),
            "no file for a fitting result"
        );
    }

    #[test]
    fn the_spill_file_is_readable_by_the_read_tool() {
        let dir = tempfile::tempdir().unwrap();
        write_many_files(dir.path(), 300);
        let page = call_in(dir.path(), serde_json::json!({"pattern": "needle"}));
        let rel = spill_path(&page).expect("spill path");
        assert!(
            rel.starts_with(".litecode/bash/grep_"),
            "spill must land where read can open it, got: {rel}"
        );
        assert!(rel.ends_with(".txt"), "got: {rel}");
        let read_path = dir.path().join(&rel);
        assert!(read_path.is_file(), "got: {rel}");
        let text = std::fs::read_to_string(&read_path).unwrap();
        let inline = page.matches(": needle").count();
        assert!(inline > 0, "the page carries lines itself: {page}");
        assert_eq!(
            text.matches(": needle").count(),
            300 - inline,
            "the file holds the matches the page left out, exactly once each"
        );
    }

    #[test]
    fn read_pages_through_the_spill_file() {
        let dir = tempfile::tempdir().unwrap();
        write_many_files(dir.path(), 300);
        let page = call_in(dir.path(), serde_json::json!({"pattern": "needle"}));
        let rel = spill_path(&page).expect("spill path");
        // The footer's wording is the recovery contract: it must still name a
        // window the read tool accepts.
        assert!(page.contains("start_line and end_line"), "got: {page}");

        // The recovery contract the footer promises: read opens the file by the
        // workspace-relative path it was given and start_line walks it, so an
        // over-budget result is never lost.
        let (first, later) = with_cwd(dir.path(), || {
            let first = crate::tools::read::ReadTool::default()
                .call(serde_json::json!({ "file_path": &rel }))
                .content;
            let later = crate::tools::read::ReadTool::default()
                .call(serde_json::json!({ "file_path": &rel, "start_line": 200 }))
                .content;
            (first, later)
        });
        assert!(
            first.contains("Remaining ") && first.contains("not shown inline"),
            "the file must say what it is, got: {first}"
        );
        assert!(
            first.matches(": needle").count() > 1,
            "the first window holds matches, got: {first}"
        );
        assert!(
            later.contains("source_file.rs"),
            "a later window still lands on real hits, got: {later}"
        );
        assert_ne!(first, later, "start_line must advance through the file");
    }

    #[test]
    fn an_inline_line_view_that_degraded_says_so() {
        // Hits that stay lines — one file, too many for one page — carry what fits
        // and say on the warning line what they left out.
        let dir = tempfile::tempdir().unwrap();
        let body: String = (0..400)
            .map(|i| format!("item{i:03} {}\n", "x".repeat(20)))
            .collect();
        write(dir.path(), "big.rs", &body);
        let result = call_result(dir.path(), serde_json::json!({"pattern": "item\\d+"}));
        assert!(
            result.content.starts_with("Found 400 matches (lines"),
            "got: {}",
            result.content
        );
        let warning = result
            .warning_status
            .expect("a truncated line view must warn");
        assert!(warning.contains(" of 400 matches"), "got: {warning}");
        assert!(warning.contains("file below"), "got: {warning}");
    }

    #[test]
    fn a_wide_single_file_result_still_shows_lines_and_names_the_rest() {
        // 400 hits in one file: the hit list is over budget, so the page carries
        // the lines that fit and the file carries the rest.
        let dir = tempfile::tempdir().unwrap();
        let body: String = (0..400)
            .map(|i| format!("item{i:03} {}\n", "x".repeat(20)))
            .collect();
        write(dir.path(), "big.rs", &body);
        let out = call_in(dir.path(), serde_json::json!({"pattern": "item\\d+"}));
        assert!(out.starts_with("Found 400 matches (lines"), "got: {out}");
        assert!(
            out.contains("     1: item000"),
            "the page shows lines, not only a count, got: {out}"
        );
        assert!(
            out.contains(
                "All 400 matches are in big.rs: narrow the pattern, or read the file itself."
            ),
            "one file has no map and no path to narrow to, got: {out}"
        );
        let inline = out.matches(": item").count();
        let rel = spill_path(&out).expect("the rest must be named");
        let spilled = std::fs::read_to_string(dir.path().join(&rel)).unwrap();
        assert_eq!(
            spilled.matches(": item").count(),
            400 - inline,
            "inline plus file is every hit, once each"
        );
    }

    // -----------------------------------------------------------------
    // 4. Boundaries
    // -----------------------------------------------------------------

    #[test]
    fn empty_workspace_reports_the_corpus_without_naming_a_knob() {
        let dir = tempfile::tempdir().unwrap();
        let out = call_in(dir.path(), serde_json::json!({"pattern": "anything"}));
        assert!(out.starts_with("No matches found"), "got: {out}");
        assert!(out.contains("excludes.json"), "got: {out}");
        assert!(!out.contains("-u"), "got: {out}");
    }

    /// Runs one call in a workspace whose exclude lists hide `hidden_dir`.
    ///
    /// A file list the workspace writes is read by the process-global excludes
    /// cache, so the body activates the lists through the crate's test hook: it
    /// serializes against other cache-mutating tests and restores the previous
    /// lists on the way out.
    fn with_excluded_dir<R>(f: impl FnOnce() -> R) -> R {
        let mut file = crate::workspace::filter::WorkspaceExcludesFile::builtin_defaults();
        file.search_exclude.push("hidden_dir".into());
        let mut out = None;
        crate::workspace::filter::with_excludes_cache_for_test(file, || {
            out = Some(f());
        });
        out.expect("cache test body ran")
    }

    #[test]
    fn an_excluded_tree_is_searched_and_the_lift_is_disclosed() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "hidden_dir/hidden.rs", "needle_lift\n");
        write(dir.path(), "visible.rs", "nothing\n");
        let path = dir.path().to_path_buf();
        let out =
            with_excluded_dir(|| call_in(&path, serde_json::json!({"pattern": "needle_lift"})));
        assert!(
            out.contains("hidden.rs"),
            "a zero-hit default search must retry without the exclude filters, got: {out}"
        );
        assert!(
            out.contains("including excluded paths"),
            "the retry must be disclosed, got: {out}"
        );
    }

    #[test]
    fn naming_an_excluded_tree_is_refused_with_the_reason() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "hidden_dir/hidden.rs", "needle\n");
        let path = dir.path().to_path_buf();
        let out = with_excluded_dir(|| {
            call_in(
                &path,
                serde_json::json!({"pattern": "needle", "path": "hidden_dir"}),
            )
        });
        assert!(out.contains("not searched"), "got: {out}");
        assert!(out.contains("search.exclude"), "got: {out}");
        assert!(out.contains("name a single file inside it"), "got: {out}");
    }

    #[test]
    fn a_lift_that_is_still_a_miss_keeps_the_plain_miss() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "hidden_dir/hidden.rs", "present\n");
        let path = dir.path().to_path_buf();
        let out = with_excluded_dir(|| {
            call_in(&path, serde_json::json!({"pattern": "absent_everywhere"}))
        });
        assert!(
            out.starts_with("No matches found."),
            "a miss after the retry stays a plain miss, got: {out}"
        );
        assert!(
            out.contains(".litecode/excludes.json"),
            "the miss still names the corpus it did not search, got: {out}"
        );
        assert!(!out.contains("searched including"), "got: {out}");
    }

    #[test]
    fn multiple_hits_in_one_file_render_each_occurrence() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.rs", "needle\nfiller\nneedle\n");
        let out = call_in(dir.path(), serde_json::json!({"pattern": "needle"}));
        assert!(out.starts_with("Found 2 matches"), "got: {out}");
        assert_eq!(out.matches("needle").count(), 2, "got: {out}");
    }

    #[test]
    fn snippet_lines_are_truncated_but_marked() {
        let dir = tempfile::tempdir().unwrap();
        let long = format!("needle {}\n", "x".repeat(400));
        write(dir.path(), "big.rs", &long);
        let out = call_in(dir.path(), serde_json::json!({"pattern": "needle"}));
        assert!(
            out.contains("(line truncated)"),
            "a cut line must say so, got: {out}"
        );
    }

    #[test]
    fn a_single_oversized_snippet_still_returns_something() {
        let dir = tempfile::tempdir().unwrap();
        // 40 hits of 400 chars each: over budget even at one snippet.
        let body: String = (0..40)
            .map(|i| format!("needle{i:02} {}\n", "y".repeat(400)))
            .collect();
        write(dir.path(), "wide.rs", &body);
        let out = call_in(dir.path(), serde_json::json!({"pattern": "needle"}));
        assert!(
            !out.is_empty(),
            "an over-budget page must still say something"
        );
        assert!(out.starts_with("Found 40 matches"), "got: {out}");
    }

    #[test]
    fn unicode_matches_render_intact() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "cn.rs", "let s = \"世界\";\n");
        let out = call_in(dir.path(), serde_json::json!({"pattern": "世界"}));
        assert!(out.contains("世界"), "got: {out}");
    }

    #[test]
    fn a_path_outside_the_mode_is_reported_not_panicking() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.rs", "needle\n");
        let out = call_in_mode(
            dir.path(),
            serde_json::json!({"pattern": "needle", "path": dir.path().join("a.rs").display().to_string()}),
            crate::workspace::ToolPathMode::All,
        );
        assert!(
            out.contains("a.rs"),
            "an absolute in-workspace path must work, got: {out}"
        );
    }

    #[test]
    fn empty_glob_value_falls_back_to_the_whole_workspace() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.rs", "needle\n");
        let out = call_in(
            dir.path(),
            serde_json::json!({"pattern": "needle", "glob": "   "}),
        );
        assert!(
            out.contains("a.rs"),
            "an empty glob is not a filter, got: {out}"
        );
    }

    #[test]
    fn grep_does_not_consult_the_text_index() {
        // The production path is the ripgrep walk; an index-accelerated search
        // would change which files count toward files_searched.
        assert!(
            include_str!("grep.rs").contains("lexical_search_with_preset"),
            "grep must search through the lexical walk"
        );
    }

    // -----------------------------------------------------------------
    // 5. Session transcripts (virtual paths)
    // -----------------------------------------------------------------

    #[test]
    fn virtual_session_grep_still_answers() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let sid = seed_session(root, "alpha\nVIRTUAL_GREP_NEEDLE here\ndelta");
        let path = crate::session::transcript_file::virtual_path_for(&sid);
        let out = call_in(
            root,
            serde_json::json!({"pattern": "VIRTUAL_GREP_NEEDLE", "path": path}),
        );
        assert!(out.contains("VIRTUAL_GREP_NEEDLE"), "got: {out}");
    }

    #[test]
    fn virtual_session_grep_miss_stays_a_plain_miss() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let sid = seed_session(root, "alpha\nbeta");
        let path = crate::session::transcript_file::virtual_path_for(&sid);
        let out = call_in(
            root,
            serde_json::json!({"pattern": "ABSENT_TOKEN", "path": path}),
        );
        assert_eq!(out, "No matches found");
    }

    fn seed_session(root: &std::path::Path, body: &str) -> String {
        let db = root.join(".litecode").join("sessions.db");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        let lease = crate::session::WorkspaceWriteLease::acquire(db.parent().unwrap()).unwrap();
        let data = crate::session::SessionData::open(&lease, &db).unwrap();
        let id = data
            .create_session(root.to_str().unwrap(), "default", None)
            .unwrap();
        data.insert_items(&id, &[crate::types::user_text(body)])
            .unwrap();
        id
    }
}

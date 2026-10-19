//! Agent `grep` tool - LexicalLane frontend.
//!
//! Search results are returned as a bounded, structured page. `cursor` and
//! `limit` select the page; `view` only changes how that page is rendered as
//! text (lines, context, or expanded). Presentation never changes page
//! membership, and grep never spills matches to a side file. Human workspace
//! Search continues to use LexicalLane via the retrieval facade. Both are disk
//! walks; they do not consult the text index.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::context_pipeline::Context;
use crate::engines::code_search::{
    LexicalMatch, LexicalQuery, enclosing_scopes, format_breadcrumb,
    lexical_search_for_agent_cancellable, lines_slice, syntax_ancestor_snippet,
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

const DEFAULT_GREP_LIMIT: usize = 20;
const MAX_GREP_LIMIT: usize = 100;
const MAX_MATCH_LINE_CHARS: usize = 500;

#[derive(Debug, Serialize)]
struct GrepMatchData {
    path: String,
    start_line: u32,
    end_line: u32,
    line_text: String,
    line_truncated: bool,
}

#[derive(Debug, Serialize)]
struct GrepData {
    matches: Vec<GrepMatchData>,
    total: usize,
    has_more: bool,
    next_cursor: Option<String>,
    files_searched: usize,
    searched_excluded: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct GrepCursor {
    offset: usize,
}

struct GrepPage {
    body: String,
    warning: Option<String>,
    data: Option<Value>,
}

impl GrepPage {
    fn ok(body: impl Into<String>) -> Self {
        Self {
            body: body.into(),
            warning: None,
            data: None,
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
    include_ignored: bool,
    view: GrepShape,
    cursor: usize,
    limit: usize,
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
                "include_ignored": {
                    "type": "boolean",
                    "description": "Also search gitignored and workspace-excluded paths. Default false. Search never retries a zero-hit result with filters lifted implicitly."
                },
                "case_sensitive": {
                    "type": "boolean",
                    "description": "Default true. Set false to ignore case."
                },
                "view": {
                    "type": "string",
                    "enum": ["lines", "context", "expanded"],
                    "description": "Presentation mode only. It never changes the returned match page. Default lines."
                },
                "cursor": {
                    "type": "string",
                    "description": "Opaque continuation cursor from a previous grep result."
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 100,
                    "description": "Maximum matches in this page. Default 20, max 100."
                }
            },
            "required": ["pattern"]
        })
    }

    fn data_schema(&self) -> Option<Value> {
        Some(serde_json::json!({
            "type": "object",
            "description": "One bounded page of content matches.",
            "properties": {
                "matches": {
                    "type": "array",
                    "description": "Matches in this page, ordered by path then start line.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "path": {
                                "type": "string",
                                "description": "Workspace-relative path when under the workspace; otherwise the searched path."
                            },
                            "start_line": {
                                "type": "integer",
                                "minimum": 1,
                                "description": "1-based first matching line."
                            },
                            "end_line": {
                                "type": "integer",
                                "minimum": 1,
                                "description": "1-based last matching line; equal to start_line for a single-line match."
                            },
                            "line_text": {
                                "type": "string",
                                "description": "Matched line text for display and light inspection; may be capped."
                            },
                            "line_truncated": {
                                "type": "boolean",
                                "description": "True when line_text was capped."
                            }
                        },
                        "required": [
                            "path",
                            "start_line",
                            "end_line",
                            "line_text",
                            "line_truncated"
                        ]
                    }
                },
                "total": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "Total matches across all pages."
                },
                "has_more": {
                    "type": "boolean",
                    "description": "True when another page is available."
                },
                "next_cursor": {
                    "type": ["string", "null"],
                    "description": "Opaque cursor for the next page; null on the last page."
                },
                "files_searched": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "Number of files searched for this page."
                },
                "searched_excluded": {
                    "type": "boolean",
                    "description": "True when the search was gated by workspace exclusions."
                }
            },
            "required": [
                "matches",
                "total",
                "has_more",
                "next_cursor",
                "files_searched",
                "searched_excluded"
            ]
        }))
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    fn execute(
        &self,
        input: Value,
        execution: ToolExecutionContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>> {
        Box::pin(async move {
            match tokio::task::spawn_blocking(move || GrepTool.call_for_execution(input, execution))
                .await
            {
                Ok(result) => result,
                Err(error) => ToolCallResult::error_with_kind(
                    crate::types::ToolErrorKind::Internal,
                    format!("grep worker failed: {error}"),
                    false,
                ),
            }
        })
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
        crate::tools::description_text(include_str!("descriptions/grep.md"))
    }

    fn validate_input(&self, input: &Value) -> std::result::Result<(), String> {
        parse_grep_options(input)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    fn timeout(&self) -> Option<u64> {
        Some(30)
    }

    fn is_cancellable(&self) -> bool {
        true
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
    let include_ignored = optional_bool(input, "include_ignored")?.unwrap_or(false);
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
        include_ignored,
        view: parse_grep_view(input)?,
        cursor: parse_grep_cursor(input)?,
        limit: parse_grep_limit(input)?,
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

fn parse_grep_view(input: &Value) -> Result<GrepShape> {
    match input.get("view") {
        None | Some(Value::Null) => Ok(GrepShape::Lines),
        Some(Value::String(raw)) => match raw.as_str() {
            "lines" => Ok(GrepShape::Lines),
            "context" => Ok(GrepShape::Context),
            "expanded" => Ok(GrepShape::Expanded),
            other => Err(LitecodeError::ToolExecution(format!(
                "parameter 'view' expected one of lines, context, expanded; got \"{other}\""
            ))),
        },
        Some(other) => Err(LitecodeError::ToolExecution(format!(
            "parameter 'view' expected a string, got {other}"
        ))),
    }
}

fn parse_grep_limit(input: &Value) -> Result<usize> {
    let Some(value) = input.get("limit").filter(|value| !value.is_null()) else {
        return Ok(DEFAULT_GREP_LIMIT);
    };
    let limit = value.as_u64().ok_or_else(|| {
        LitecodeError::ToolExecution("parameter 'limit' expected an integer".into())
    })? as usize;
    if !(1..=MAX_GREP_LIMIT).contains(&limit) {
        return Err(LitecodeError::ToolExecution(format!(
            "parameter 'limit' must be between 1 and {MAX_GREP_LIMIT}"
        )));
    }
    Ok(limit)
}

fn parse_grep_cursor(input: &Value) -> Result<usize> {
    let Some(value) = input.get("cursor").filter(|value| !value.is_null()) else {
        return Ok(0);
    };
    let raw = value.as_str().ok_or_else(|| {
        LitecodeError::ToolExecution("parameter 'cursor' expected a string".into())
    })?;
    decode_grep_cursor(raw)
}

fn encode_grep_cursor(offset: usize) -> String {
    let bytes = serde_json::to_vec(&GrepCursor { offset }).unwrap_or_default();
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn decode_grep_cursor(raw: &str) -> Result<usize> {
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(raw)
        .map_err(|_| LitecodeError::ToolExecution("invalid grep cursor".into()))?;
    let cursor: GrepCursor = serde_json::from_slice(&bytes)
        .map_err(|_| LitecodeError::ToolExecution("invalid grep cursor".into()))?;
    Ok(cursor.offset)
}

fn cap_match_line(text: &str) -> (String, bool) {
    let without_newline = text.trim_end_matches(['\n', '\r']);
    let mut chars = without_newline.chars();
    let kept: String = chars.by_ref().take(MAX_MATCH_LINE_CHARS).collect();
    if chars.next().is_some() {
        (format!("{kept}..."), true)
    } else {
        (kept, false)
    }
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
                let mut result = ToolCallResult::ok(page.body);
                if let Some(data) = page.data {
                    result = result.with_data(data);
                }
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
    if page.data.is_none() {
        page.data = Some(empty_grep_data(options.include_ignored));
    }
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
    if !file_scoped && !resolved.is_dir() {
        return Err(LitecodeError::ToolExecution(format!(
            "path does not exist: {}",
            resolved.display()
        )));
    }

    // Always walk relative to the workspace so match paths compose with read,
    // glob, and edit. The requested path is a scope, not the path namespace.
    let root = workspace_root.clone();
    let search_scope = input["path"]
        .as_str()
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(|_| resolved.clone());

    if file_scoped && looks_binary(&resolved) {
        return Ok(GrepPage::ok(format!(
            "path '{resolved_display}' is not searched: binary file."
        )));
    }

    let query = LexicalQuery {
        pattern: options.query_pattern(),
        root: root.clone(),
        path: search_scope,
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

    let preset = if options.include_ignored {
        FilterPreset::NoIgnore
    } else {
        FilterPreset::Search
    };
    let outcome = lexical_search_for_agent_cancellable(
        &query,
        preset,
        workspace_root,
        Some(&execution.cancel),
    )?;
    if !outcome.matches.is_empty() {
        let searched = outcome.files_searched;
        let matches = sort_grep_matches_vec(outcome.matches);
        return render_matches(
            workspace_root,
            &matches,
            options.include_ignored,
            Some(searched),
            options,
        );
    }

    if let Some(ref pat) = options.glob
        && outcome.files_searched == 0
    {
        return Ok(GrepPage::ok(format_glob_empty(pat, &resolved_display)));
    }
    if outcome.files_searched > 0 {
        return Ok(GrepPage::ok(format_no_hit(
            outcome.files_searched,
            options.include_ignored,
        )));
    }
    if file_scoped {
        return Ok(GrepPage::ok(format!(
            "No matches found (path '{resolved_display}' was not searched)."
        )));
    }
    Ok(GrepPage::ok(format_corpus_empty(options.include_ignored)))
}

fn format_no_hit(files_searched: usize, include_ignored: bool) -> String {
    if include_ignored {
        format!(
            "No matches found ({}; including excluded paths).",
            searched_scope(files_searched)
        )
    } else {
        format!(
            "No matches found ({}). {}",
            searched_scope(files_searched),
            empty_discovery_hint()
        )
    }
}

/// `searched N files`, with the count in the right number.
fn searched_scope(searched: usize) -> String {
    let plural = if searched == 1 { "" } else { "s" };
    format!("searched {searched} file{plural}")
}

fn format_corpus_empty(include_ignored: bool) -> String {
    if include_ignored {
        "No matches found. Searched including gitignored and workspace-excluded paths.".to_string()
    } else {
        format!("No matches found. {}", empty_discovery_hint())
    }
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

fn empty_grep_data(include_ignored: bool) -> Value {
    serde_json::to_value(GrepData {
        matches: Vec::new(),
        total: 0,
        has_more: false,
        next_cursor: None,
        files_searched: 0,
        searched_excluded: include_ignored,
    })
    .unwrap_or(Value::Null)
}

fn display_search_path(workspace_root: &Path, resolved: &Path) -> String {
    cheap_rel_under(workspace_root, resolved)
        .map(|rel| rel.replace('\\', "/"))
        .filter(|rel| !rel.is_empty())
        .unwrap_or_else(|| resolved.display().to_string())
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
    finish_virtual_grep_matches(matches, &execution.workspace_root, options)
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
    finish_virtual_grep_matches(matches, &execution.workspace_root, options)
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
    options: &GrepOptions,
) -> Result<GrepPage> {
    let mut matches = matches;
    sort_grep_matches(&mut matches);
    if matches.is_empty() {
        return Ok(GrepPage::ok("No matches found"));
    }
    render_matches(
        workspace_root,
        &matches,
        options.include_ignored,
        None,
        options,
    )
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
    gated: bool,
    files_searched: Option<usize>,
    options: &GrepOptions,
) -> Result<GrepPage> {
    let total = matches.len();
    if options.cursor > total {
        return Err(LitecodeError::ToolExecution(format!(
            "cursor offset {} exceeds total matches {}",
            options.cursor, total
        )));
    }

    let page_end = options.cursor.saturating_add(options.limit).min(total);
    let page_matches = &matches[options.cursor..page_end];
    let has_more = page_end < total;
    let next_cursor = has_more.then(|| encode_grep_cursor(page_end));

    let data_matches: Vec<GrepMatchData> = page_matches
        .iter()
        .map(|m| {
            let (line_text, line_truncated) = cap_match_line(&m.line_text);
            GrepMatchData {
                path: m.path.clone(),
                start_line: m.start_line,
                end_line: m.end_line,
                line_text,
                line_truncated,
            }
        })
        .collect();
    let data = GrepData {
        matches: data_matches,
        total,
        has_more,
        next_cursor: next_cursor.clone(),
        files_searched: files_searched.unwrap_or(0),
        searched_excluded: gated,
    };
    let data_value = serde_json::to_value(data)
        .map_err(|error| LitecodeError::ToolExecution(format!("serialize grep data: {error}")))?;

    if total == 0 {
        return Ok(GrepPage {
            body: "No matches found".into(),
            warning: None,
            data: Some(data_value),
        });
    }
    if page_matches.is_empty() {
        return Ok(GrepPage {
            body: format!("No matches on this page (total {total})."),
            warning: None,
            data: Some(data_value),
        });
    }

    let rendered = match options.view {
        GrepShape::Expanded => format_snippet_body(root, page_matches, GrepShape::Expanded),
        GrepShape::Context => format_snippet_body(root, page_matches, GrepShape::Context),
        GrepShape::Lines => format_compact_body(page_matches),
    };

    let scope = files_searched
        .map(|searched| format!("; {}", searched_scope(searched)))
        .unwrap_or_default();
    let gated_scope = if gated {
        if files_searched.is_some() {
            "; including excluded paths"
        } else {
            "; searched including excluded paths"
        }
    } else {
        ""
    };
    let mut body = format!(
        "Found {total} matches ({}; page {}..{} of {total}{scope}{gated_scope}):
{rendered}",
        options.view.label(),
        options.cursor,
        page_end.saturating_sub(1),
    );
    if let Some(cursor) = &next_cursor {
        body.push_str(&format!(
            "\n\nMore matches available. Continue with cursor: {cursor}"
        ));
    }

    debug_assert!(!page_matches.is_empty());
    Ok(GrepPage {
        body,
        warning: None,
        data: Some(data_value),
    })
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

    /// Thousands of hits spread over many files exercise pagination.
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
    // 1. Public grep surface
    // -----------------------------------------------------------------

    fn grep_data(result: &ToolCallResult) -> Value {
        result
            .data
            .clone()
            .expect("grep must return structured data")
    }

    #[test]
    fn schema_exposes_pagination_and_view_parameters() {
        assert_eq!(GrepTool.timeout(), Some(30));
        assert!(GrepTool.is_cancellable());
        let schema = GrepTool.schema();
        let props = schema["properties"].as_object().unwrap();
        for expected in [
            "case_sensitive",
            "cursor",
            "glob",
            "include_ignored",
            "limit",
            "path",
            "pattern",
            "view",
        ] {
            assert!(props.contains_key(expected), "missing {expected}");
        }
        assert_eq!(schema["required"], serde_json::json!(["pattern"]));
        assert_eq!(
            schema["properties"]["view"]["enum"],
            serde_json::json!(["lines", "context", "expanded"])
        );
    }

    #[test]
    fn page_is_bounded_and_cursor_continues() {
        let dir = tempfile::tempdir().unwrap();
        write_many_files(dir.path(), 25);

        let first = call_result(dir.path(), serde_json::json!({"pattern": "needle"}));
        assert_eq!(first.level, crate::types::ToolSignalLevel::Ok);
        let first_data = grep_data(&first);
        assert_eq!(first_data["total"], 25);
        assert_eq!(first_data["matches"].as_array().unwrap().len(), 20);
        assert_eq!(first_data["has_more"], true);
        let cursor = first_data["next_cursor"].as_str().unwrap().to_string();

        let second = call_result(
            dir.path(),
            serde_json::json!({"pattern": "needle", "cursor": cursor}),
        );
        let second_data = grep_data(&second);
        assert_eq!(second_data["total"], 25);
        assert_eq!(second_data["matches"].as_array().unwrap().len(), 5);
        assert_eq!(second_data["has_more"], false);
        assert!(second_data["next_cursor"].is_null());
    }

    #[test]
    fn view_changes_text_not_structured_page() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.rs", NESTED_RS);

        let lines = call_result(
            dir.path(),
            serde_json::json!({"pattern": "needle_here", "view": "lines"}),
        );
        let expanded = call_result(
            dir.path(),
            serde_json::json!({"pattern": "needle_here", "view": "expanded"}),
        );

        assert_eq!(grep_data(&lines), grep_data(&expanded));
        assert!(
            !lines.content.contains("fn main"),
            "lines view must stay compact"
        );
        assert!(
            expanded.content.contains("fn main"),
            "expanded view must include enclosing code, got: {}",
            expanded.content
        );
    }

    #[test]
    fn wide_search_never_writes_a_spill_file() {
        let dir = tempfile::tempdir().unwrap();
        write_many_files(dir.path(), 30);
        let result = call_result(dir.path(), serde_json::json!({"pattern": "needle"}));
        assert_eq!(grep_data(&result)["has_more"], true);

        let bash_dir = dir.path().join(".litecode").join("bash");
        if bash_dir.is_dir() {
            let spill = std::fs::read_dir(bash_dir)
                .unwrap()
                .filter_map(std::result::Result::ok)
                .any(|entry| entry.file_name().to_string_lossy().starts_with("grep_"));
            assert!(!spill, "grep must not spill matches to a file");
        }
    }

    #[test]
    fn invalid_view_and_limit_are_rejected() {
        assert!(
            GrepTool
                .validate_input(&serde_json::json!({"pattern": "x", "view": "auto"}))
                .is_err()
        );
        assert!(
            GrepTool
                .validate_input(&serde_json::json!({"pattern": "x", "limit": 0}))
                .is_err()
        );
        assert!(
            GrepTool
                .validate_input(&serde_json::json!({"pattern": "x", "limit": 101}))
                .is_err()
        );
    }

    #[test]
    fn data_line_text_is_bounded_and_marked() {
        let dir = tempfile::tempdir().unwrap();
        let long = format!("needle {}\n", "x".repeat(600));
        write(dir.path(), "long.rs", &long);
        let result = call_result(dir.path(), serde_json::json!({"pattern": "needle"}));
        let data = grep_data(&result);
        let hit = &data["matches"][0];
        assert_eq!(hit["line_truncated"], true);
        assert!(hit["line_text"].as_str().unwrap().ends_with("..."));
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
        assert_eq!(
            pattern_mode("println!(\"{}\")", true),
            PatternMode::BraceText
        );
        assert_eq!(pattern_mode("found[", true), PatternMode::Literal);
        assert_eq!(escape_bare_braces("ModelRequest {"), "ModelRequest \\{");
        assert_eq!(escape_bare_braces(r"a{2,3}"), r"a\{2,3}");
        assert_eq!(escape_bare_braces(r"a\{"), r"a\{");
        assert_eq!(escape_bare_braces(r"a\\{"), r"a\\\{");
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
    fn excluded_tree_requires_explicit_include_ignored() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "hidden_dir/hidden.rs", "needle_lift\n");
        write(dir.path(), "visible.rs", "nothing\n");
        let path = dir.path().to_path_buf();

        let out =
            with_excluded_dir(|| call_in(&path, serde_json::json!({"pattern": "needle_lift"})));
        assert!(
            !out.contains("hidden.rs"),
            "zero-hit must not lift excludes implicitly: {out}"
        );
        assert!(out.starts_with("No matches found"), "got: {out}");
        assert!(out.contains("excludes.json"), "got: {out}");

        let included = with_excluded_dir(|| {
            call_in(
                &path,
                serde_json::json!({"pattern": "needle_lift", "include_ignored": true}),
            )
        });
        assert!(included.contains("hidden.rs"), "got: {included}");
        assert!(
            included.contains("including excluded paths"),
            "got: {included}"
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
    fn include_ignored_miss_names_the_expanded_corpus() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "hidden_dir/hidden.rs", "present\n");
        let path = dir.path().to_path_buf();
        let out = with_excluded_dir(|| {
            call_in(
                &path,
                serde_json::json!({"pattern": "absent_everywhere", "include_ignored": true}),
            )
        });
        assert!(
            out.starts_with("No matches found") && out.contains("including excluded paths"),
            "got: {out}"
        );
        assert!(!out.contains("Default filters may hide"), "got: {out}");
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
    fn grep_does_not_consult_the_path_index() {
        let src = include_str!("grep.rs");
        let prod = src.split("mod tests").next().expect("production source");
        assert!(
            prod.contains("lexical_search_for_agent_cancellable"),
            "grep must search through the lexical walk"
        );
        assert!(
            !prod.contains("path_index") && !prod.contains("text_index"),
            "grep must not read the mention path catalog"
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

    #[test]
    fn data_schema_accepts_structured_page() {
        let dir = tempfile::tempdir().unwrap();
        write_many_files(dir.path(), 3);
        let result = call_result(dir.path(), serde_json::json!({"pattern": "needle"}));
        let data = grep_data(&result);
        let schema = GrepTool.data_schema().expect("grep data schema");
        crate::tool::schema_validate(&schema, &data)
            .unwrap_or_else(|error| panic!("grep data does not match its schema: {error}: {data}"));
    }
}

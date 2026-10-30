//! Agent `grep` tool.
//!
//! Bounded content search. Result shape is `{ hits, message }`, plus
//! `truncated: true` when a limit or the 8K-token data guard removed context
//! or hits. Hits are `{ path, line, text }` with 1-based line numbers; when
//! requested, `before` and `after` add exact `{ line, text }` context arrays.
//! Empty-result and truncation guidance live in `message` only — no cursor,
//! page, next, or offset fields.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;

use crate::engines::code_search::{LexicalQuery, lexical_search_for_agent_cancellable};
use crate::tool::Tool;
use crate::tool::file_identity;
use crate::tool::trait_::ToolExecutionContext;
use crate::types::{LitecodeError, Result, ToolCallResult};
use crate::workspace::filter::{
    FilterPreset, PathGlobMatcher, WalkOptions, cheap_rel_under, compile_include_patterns,
    ignored_discovery_message, looks_binary, path_matches_include, private_knowledge_walker,
    split_glob_include_exclude, walk_builder_with,
};

const DEFAULT_GREP_LIMIT: usize = 40;
const MAX_GREP_LIMIT: usize = 100;
const DEFAULT_CONTEXT_LINES: usize = 0;
const MAX_CONTEXT_LINES: usize = 5;
const MAX_MATCH_LINE_CHARS: usize = 500;
/// The serialized grep `data` object is capped at 8K cl100k tokens. Context is
/// dropped first; only then are hits removed.
const MAX_GREP_DATA_TOKENS: usize = 8_000;
/// Backstop on matches examined while filling one call. Production asks for
/// at most `limit + 1`, so this fires only when a caller tightens it.
const GREP_EXAMINE_CAP: usize = 10_000;

#[derive(Debug, Clone)]
struct GrepContextLine {
    line: u32,
    text: String,
}

/// One match. Context never reaches the wire as its own field: it is folded
/// into `text` (see [`hit_row`]) so every hit stays `{ path, line, text }` and
/// a row set renders as one table instead of nested arrays.
#[derive(Debug, Clone)]
struct GrepHit {
    path: String,
    line: u32,
    text: String,
    before: Vec<GrepContextLine>,
    after: Vec<GrepContextLine>,
}

#[derive(Debug)]
struct GrepOptions {
    pattern: String,
    glob: Option<String>,
    include: Option<Vec<PathGlobMatcher>>,
    exclude: Option<Vec<PathGlobMatcher>>,
    ignore_case: bool,
    literal: bool,
    limit: usize,
    before: usize,
    after: usize,
}

struct Candidate {
    abs: PathBuf,
    display: String,
}

pub struct GrepTool;

impl Tool for GrepTool {
    fn name(&self) -> &str {
        "grep"
    }

    fn surface(&self) -> crate::tool::ToolSurface {
        crate::tool::ToolSurface::Code
    }

    fn effects(
        &self,
        input: &Value,
        path_mode: crate::workspace::ToolPathMode,
        workspace_root: &std::path::Path,
    ) -> crate::tool::resource_gate::ToolEffects {
        crate::tool::resource_gate::ToolEffects::read(crate::tool::resource_gate::scan_key(
            workspace_root,
            input.get("path").and_then(Value::as_str),
            path_mode,
        ))
    }

    fn execute(
        &self,
        input: Value,
        execution: ToolExecutionContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>> {
        Box::pin(async move {
            match tokio::task::spawn_blocking(move || dispatch(input, execution, GREP_EXAMINE_CAP))
                .await
            {
                Ok(result) => result,
                Err(error) => ToolCallResult::error_with_kind(
                    crate::types::ToolErrorKind::Internal,
                    format!("grep worker failed: {error}"),
                    false,
                )
                .with_error_code("internal"),
            }
        })
    }

    fn call_inner(&self, input: Value) -> ToolCallResult {
        dispatch(
            input,
            ToolExecutionContext {
                path_mode: crate::workspace::ToolPathMode::All,
                workspace_root: crate::config::workspace::workspace_root_lap(),
                call_id: String::new(),
                cancel: tokio_util::sync::CancellationToken::new(),
                output_limit: usize::MAX,
                session_id: String::new(),
                session: None,
            },
            GREP_EXAMINE_CAP,
        )
    }

    fn validate_input(&self, input: &Value) -> std::result::Result<(), String> {
        parse_grep_options(input)
            .map(|_| ())
            .map_err(|error| match error {
                LitecodeError::ToolExecution(message) => message,
                other => other.to_string(),
            })
    }

    fn timeout(&self) -> Option<Duration> {
        Some(Duration::from_secs(30))
    }

    fn is_cancellable(&self) -> bool {
        true
    }
}

fn dispatch(input: Value, execution: ToolExecutionContext, examine_cap: usize) -> ToolCallResult {
    if let Some(rejected) = reject_invalid_regex(&input) {
        return rejected;
    }
    let options = match parse_grep_options(&input) {
        Ok(options) => options,
        Err(error) => return crate::tools::file_path::litecode_error(error),
    };
    match search(&input, &options, &execution, examine_cap) {
        Ok(result) => result,
        Err(error) => crate::tools::file_path::litecode_error(error),
    }
}

fn parse_grep_options(input: &Value) -> Result<GrepOptions> {
    let pattern = crate::tool::require_nonempty_string(input, "pattern")
        .map_err(LitecodeError::ToolExecution)?
        .to_string();
    let ignore_case = optional_bool(input, "ignore_case")?.unwrap_or(false);
    let literal = optional_bool(input, "literal")?.unwrap_or(false);
    let glob = optional_string(input, "glob")?;
    let (include, exclude) = match glob.as_deref() {
        Some(raw) => {
            let (include, exclude) = split_glob_include_exclude(raw);
            (compile_glob_list(include)?, compile_glob_list(exclude)?)
        }
        None => (None, None),
    };
    Ok(GrepOptions {
        pattern,
        glob,
        include,
        exclude,
        ignore_case,
        literal,
        limit: parse_limit(input)?,
        before: parse_context(input, "before")?,
        after: parse_context(input, "after")?,
    })
}

fn compile_glob_list(raw: Option<String>) -> Result<Option<Vec<PathGlobMatcher>>> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    Ok(Some(compile_include_patterns(&raw)?))
}

fn parse_limit(input: &Value) -> Result<usize> {
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

fn parse_context(input: &Value, name: &str) -> Result<usize> {
    let Some(value) = input.get(name).filter(|value| !value.is_null()) else {
        return Ok(DEFAULT_CONTEXT_LINES);
    };
    let lines = value.as_u64().ok_or_else(|| {
        LitecodeError::ToolExecution(format!("parameter '{name}' expected an integer"))
    })?;
    if lines > MAX_CONTEXT_LINES as u64 {
        return Err(LitecodeError::ToolExecution(format!(
            "parameter '{name}' must be between 0 and {MAX_CONTEXT_LINES}"
        )));
    }
    Ok(lines as usize)
}

fn optional_string(input: &Value, name: &str) -> Result<Option<String>> {
    match input.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if value.trim().is_empty() => Err(LitecodeError::ToolExecution(
            format!("parameter '{name}' must not be blank"),
        )),
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

fn reject_invalid_regex(input: &Value) -> Option<ToolCallResult> {
    if input.get("literal").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let pattern = input.get("pattern").and_then(Value::as_str)?;
    if pattern.is_empty() {
        return None;
    }
    let ignore_case = match input.get("ignore_case") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(value)) => *value,
        Some(_) => return None,
    };
    match regex::RegexBuilder::new(pattern)
        .case_insensitive(ignore_case)
        .build()
    {
        Ok(_) => None,
        Err(error) => Some(invalid_regex_result(pattern, &error)),
    }
}

fn invalid_regex_result(pattern: &str, error: &regex::Error) -> ToolCallResult {
    let position = regex_error_position(&error.to_string());
    let details = serde_json::json!({
        "pattern": pattern,
        "regex_error": error.to_string(),
        "position": position,
    });
    let message = format!("invalid regular expression: {error}");
    crate::tool::outcome::ToolOutcome::<Value>::failure("invalid_regex", message.clone(), false)
        .with_error_details(details)
        .into_call_result(message)
}

fn regex_error_position(rendered: &str) -> Option<usize> {
    let lines: Vec<&str> = rendered.lines().collect();
    for pair in lines.windows(2) {
        let marker = pair[1].trim_end();
        if !marker.contains('^') || !marker.chars().all(|ch| ch == ' ' || ch == '^') {
            continue;
        }
        let column = marker.find('^')?;
        let shown = pair[0];
        let indent = shown.len() - shown.trim_start().len();
        if column >= indent {
            return Some(column - indent);
        }
    }
    None
}

fn search(
    input: &Value,
    options: &GrepOptions,
    execution: &ToolExecutionContext,
    examine_cap: usize,
) -> Result<ToolCallResult> {
    let raw_path = input
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty());
    if let Some(raw_path) = raw_path {
        if crate::session::transcript_file::is_virtual_session_path(raw_path) {
            return grep_virtual_file(raw_path, options, execution, examine_cap);
        }
        if crate::session::transcript_file::is_virtual_session_dir(raw_path) {
            return grep_virtual_dir(options, execution, examine_cap);
        }
    }

    let workspace = &execution.workspace_root;
    let resolved = match raw_path {
        Some(raw_path) => {
            match crate::workspace::resolve_agent(workspace, raw_path, execution.path_mode) {
                Ok(path) => path,
                Err(error) => return Ok(crate::tools::file_path::tool_path_error(error)),
            }
        }
        None => crate::config::path::canon_abs_lossy(workspace),
    };
    let outside = !crate::config::path::is_under(&resolved, workspace);
    let file_scoped = resolved.is_file();
    if !file_scoped && !resolved.is_dir() {
        return Ok(crate::tools::file_path::coded_error(
            "not_found",
            format!("path does not exist: {}", resolved.display()),
        ));
    }
    if file_scoped && looks_binary(&resolved) {
        return Ok(finish_hits(
            Vec::new(),
            "Binary file skipped; grep only searches text files.",
            false,
        ));
    }
    if !file_scoped
        && raw_path.is_some()
        && ignored_discovery_message(workspace, &resolved).is_some()
    {
        return Ok(finish_hits(
            Vec::new(),
            "Path is hidden by default search filters (.gitignore / workspace excludes).",
            false,
        ));
    }

    let preset = FilterPreset::Search;
    let candidates = list_candidates(
        workspace,
        &resolved,
        outside,
        file_scoped,
        raw_path.unwrap_or(""),
        options,
        preset,
        &execution.cancel,
    )?;

    if options.glob.is_some() && candidates.is_empty() {
        return Ok(finish_hits(
            Vec::new(),
            "No files matched the glob filter.",
            false,
        ));
    }

    let scan = collect_matches(
        workspace,
        outside,
        &candidates,
        options,
        preset,
        examine_cap,
        &execution.cancel,
    )?;
    if scan.hits.is_empty() && !scan.truncated {
        if file_scoped && !scan.opened {
            return Ok(finish_hits(
                Vec::new(),
                "File could not be searched.",
                false,
            ));
        }
        if scan.opened || !candidates.is_empty() {
            return Ok(finish_hits(Vec::new(), "No matches found.", false));
        }
        return Ok(finish_hits(Vec::new(), "No files to search.", false));
    }

    let message = if scan.truncated {
        limit_truncation_message(scan.hits.len())
    } else if scan.line_capped {
        "One or more match or context lines were truncated in text.".into()
    } else {
        String::new()
    };
    Ok(finish_hits(scan.hits, message, scan.truncated))
}

struct Scan {
    hits: Vec<GrepHit>,
    truncated: bool,
    opened: bool,
    line_capped: bool,
}

fn collect_matches(
    workspace: &Path,
    outside: bool,
    candidates: &[Candidate],
    options: &GrepOptions,
    preset: FilterPreset,
    examine_cap: usize,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<Scan> {
    let limit = options.limit;
    let target = (limit + 1).min(examine_cap.max(1));
    let mut kept = Vec::new();
    let mut opened = false;
    let mut last_file_full = false;
    let mut files_left = false;
    let mut line_capped = false;
    for (index, candidate) in candidates.iter().enumerate() {
        if cancel.is_cancelled() {
            return Err(LitecodeError::ToolExecution("search cancelled".into()));
        }
        if kept.len() >= target {
            files_left = true;
            break;
        }
        let fetch = target - kept.len();
        let (hits, file_opened, capped) = search_file(
            workspace, outside, candidate, options, preset, 0, fetch, cancel,
        )?;
        if file_opened {
            opened = true;
        }
        if capped {
            line_capped = true;
        }
        last_file_full = hits.len() == fetch;
        kept.extend(hits);
        if kept.len() >= target && index + 1 < candidates.len() {
            files_left = true;
        }
    }
    let hit_cap = examine_cap < limit + 1 && kept.len() >= examine_cap;
    let has_extra = kept.len() > limit;
    if has_extra {
        kept.truncate(limit);
    }
    let truncated = has_extra || (hit_cap && (last_file_full || files_left));
    Ok(Scan {
        hits: kept,
        truncated,
        opened,
        line_capped,
    })
}

fn search_file(
    workspace: &Path,
    outside: bool,
    candidate: &Candidate,
    options: &GrepOptions,
    preset: FilterPreset,
    min_line: u64,
    fetch: usize,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<(Vec<GrepHit>, bool, bool)> {
    let root = if outside {
        candidate
            .abs
            .parent()
            .unwrap_or(candidate.abs.as_path())
            .to_path_buf()
    } else {
        workspace.to_path_buf()
    };
    let query = LexicalQuery {
        pattern: options.pattern.clone(),
        root,
        path: Some(candidate.abs.clone()),
        case_sensitive: !options.ignore_case,
        whole_word: false,
        is_regex: !options.literal,
        include: None,
        exclude: None,
        multiline: false,
        max_matches: fetch.max(1),
        before_context: options.before,
        after_context: options.after,
        min_line: u32::try_from(min_line).unwrap_or(u32::MAX),
    };
    let outcome = lexical_search_for_agent_cancellable(&query, preset, workspace, Some(cancel))?;
    let opened = outcome.files_searched > 0 || !outcome.matches.is_empty();
    let mut line_capped = false;
    let hits = outcome
        .matches
        .into_iter()
        .map(|hit| {
            let (text, capped) = cap_match_line(&hit.line_text);
            if capped {
                line_capped = true;
            }
            let before_start = hit
                .start_line
                .saturating_sub(hit.context_before.len() as u32)
                .max(1);
            let before = context_lines(hit.context_before, before_start, &mut line_capped);
            let after = context_lines(
                hit.context_after,
                hit.end_line.saturating_add(1),
                &mut line_capped,
            );
            GrepHit {
                path: candidate.display.clone(),
                line: hit.start_line,
                text,
                before,
                after,
            }
        })
        .collect();
    Ok((hits, opened, line_capped))
}
fn list_candidates(
    workspace: &Path,
    search_root: &Path,
    outside: bool,
    file_scoped: bool,
    requested: &str,
    options: &GrepOptions,
    preset: FilterPreset,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<Vec<Candidate>> {
    let mut found = Vec::new();
    let mut seen = HashSet::new();
    if file_scoped {
        push_candidate(
            &mut found,
            &mut seen,
            workspace,
            search_root,
            outside,
            true,
            requested,
            options,
        );
        found.sort_by(|a, b| {
            crate::workspace::glob_hit_key(&a.display)
                .cmp(&crate::workspace::glob_hit_key(&b.display))
        });
        return Ok(found);
    }

    let mut walk_opts = match &options.include {
        Some(matchers) => WalkOptions::with_file_include(matchers.clone()),
        None => WalkOptions::default(),
    };
    walk_opts.allow_knowledge_root = true;
    let mut builder = walk_builder_with(search_root, preset, walk_opts);
    let knowledge = workspace.join(".litecode").join("knowledge");
    if knowledge.is_dir() && cheap_rel_under(&knowledge, search_root).is_some() {
        builder
            .git_ignore(false)
            .git_global(false)
            .git_exclude(false);
    }
    for entry in builder.build() {
        if cancel.is_cancelled() {
            return Err(LitecodeError::ToolExecution("search cancelled".into()));
        }
        let Ok(entry) = entry else {
            continue;
        };
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        push_candidate(
            &mut found,
            &mut seen,
            workspace,
            entry.path(),
            outside,
            false,
            requested,
            options,
        );
    }
    if !outside
        && knowledge.is_dir()
        && cheap_rel_under(search_root, &knowledge).is_some()
        && cheap_rel_under(&knowledge, search_root).is_none()
        && let Some(extra) = private_knowledge_walker(workspace, preset, None)
    {
        for entry in extra.build().flatten() {
            if cancel.is_cancelled() {
                return Err(LitecodeError::ToolExecution("search cancelled".into()));
            }
            if !entry.file_type().is_some_and(|kind| kind.is_file()) {
                continue;
            }
            push_candidate(
                &mut found,
                &mut seen,
                workspace,
                entry.path(),
                outside,
                false,
                requested,
                options,
            );
        }
    }
    found.sort_by(|a, b| {
        crate::workspace::glob_hit_key(&a.display).cmp(&crate::workspace::glob_hit_key(&b.display))
    });
    Ok(found)
}

fn push_candidate(
    found: &mut Vec<Candidate>,
    seen: &mut HashSet<String>,
    workspace: &Path,
    abs: &Path,
    outside: bool,
    file_scoped: bool,
    requested: &str,
    options: &GrepOptions,
) {
    let display = if outside && file_scoped {
        requested.to_string()
    } else if outside {
        file_identity::absolute_path_string(abs)
    } else {
        cheap_rel_under(workspace, abs)
            .filter(|rel| !rel.is_empty())
            .unwrap_or_else(|| {
                abs.file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("")
                    .to_string()
            })
            .replace('\\', "/")
    };
    if !glob_allows(
        &display,
        options.include.as_deref(),
        options.exclude.as_deref(),
    ) {
        return;
    }
    if !seen.insert(display.clone()) {
        return;
    }
    found.push(Candidate {
        abs: abs.to_path_buf(),
        display,
    });
}

fn glob_allows(
    display: &str,
    include: Option<&[PathGlobMatcher]>,
    exclude: Option<&[PathGlobMatcher]>,
) -> bool {
    let slash = display.replace('\\', "/");
    let name = slash.rsplit('/').next().unwrap_or(slash.as_str());
    let matches = |matchers: &[PathGlobMatcher]| {
        path_matches_include(&slash, matchers)
            || (name != slash.as_str() && path_matches_include(name, matchers))
    };
    if exclude.is_some_and(matches) {
        return false;
    }
    match include {
        Some(include) => matches(include),
        None => true,
    }
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

fn context_lines(
    raw: Vec<String>,
    first_line: u32,
    line_capped: &mut bool,
) -> Vec<GrepContextLine> {
    raw.into_iter()
        .enumerate()
        .map(|(offset, line)| {
            let (text, capped) = cap_match_line(&line);
            if capped {
                *line_capped = true;
            }
            GrepContextLine {
                line: first_line.saturating_add(offset as u32),
                text,
            }
        })
        .collect()
}

fn limit_truncation_message(shown: usize) -> String {
    if shown >= MAX_GREP_LIMIT {
        format!(
            "Showing first {shown} hits; maximum limit reached. Narrow pattern/path for fewer matches."
        )
    } else {
        format!(
            "Showing first {shown} hits; more matches exist. Narrow pattern/path for fewer matches, or raise limit to see more."
        )
    }
}

fn join_message(base: &str, detail: &str) -> String {
    if base.trim().is_empty() {
        detail.to_string()
    } else {
        format!("{base} {detail}")
    }
}

fn context_guard_message(base: &str) -> String {
    join_message(
        base,
        "Context was omitted to keep the result within 8,000 cl100k tokens; `text` keeps the matching line only. Narrow pattern/path for fewer matches, or raise limit when more hits are needed.",
    )
}

fn hits_guard_message(base: &str, shown: usize, total: usize, context_omitted: bool) -> String {
    let omission = if context_omitted {
        " after context was omitted"
    } else {
        ""
    };
    join_message(
        base,
        &format!(
            "Output capped at 8,000 cl100k tokens: showing first {shown} of {total} hits{omission}. Narrow pattern/path for fewer matches, or raise limit when more hits are needed."
        ),
    )
}

/// Wire row: `{ path, line, text }`, always. With context, `text` is one block
/// in ripgrep notation (`12- before`, `13: match`, `14+ after`) so the hit set
/// stays flat and table-renderable.
fn hit_row(hit: &GrepHit, with_context: bool) -> Value {
    let text = if with_context && !(hit.before.is_empty() && hit.after.is_empty()) {
        context_block(hit)
    } else {
        hit.text.clone()
    };
    serde_json::json!({
        "path": hit.path,
        "line": hit.line,
        "text": text,
    })
}

/// One block row: `12- text`, `13: text`, `14+ text`. An empty line keeps the
/// marker only (`12-`), like ripgrep, so a blank source line leaves no
/// trailing whitespace in the cell.
fn block_row(number: u32, marker: char, text: &str) -> String {
    if text.is_empty() {
        format!("{number}{marker}")
    } else {
        format!("{number}{marker} {text}")
    }
}

fn context_block(hit: &GrepHit) -> String {
    let mut out = String::new();
    for line in &hit.before {
        out.push_str(&block_row(line.line, '-', &line.text));
        out.push('\n');
    }
    out.push_str(&block_row(hit.line, ':', &hit.text));
    for line in &hit.after {
        out.push('\n');
        out.push_str(&block_row(line.line, '+', &line.text));
    }
    out
}

fn hits_data(hits: &[GrepHit], message: &str, truncated: bool, with_context: bool) -> Value {
    let rows: Vec<Value> = hits.iter().map(|hit| hit_row(hit, with_context)).collect();
    let mut data = serde_json::json!({
        "hits": rows,
        "message": message,
    });
    if truncated {
        data["truncated"] = Value::Bool(true);
    }
    data
}

fn data_fits(data: &Value) -> bool {
    serde_json::to_string(data).is_ok_and(|serialized| {
        crate::session::count_text_tokens(&serialized) <= MAX_GREP_DATA_TOKENS
    })
}

fn finish_hits(
    mut hits: Vec<GrepHit>,
    message: impl Into<String>,
    truncated: bool,
) -> ToolCallResult {
    let base_message = message.into();
    let initial = hits_data(&hits, &base_message, truncated, true);
    if data_fits(&initial) {
        // Code surface: facts in data; content empty (same as write/edit present).
        return crate::tool::outcome::ok_data(initial);
    }

    // Degrade in the agreed order: re-render every hit without its context
    // block first, so `text` keeps only the matching line.
    let had_context = hits
        .iter()
        .any(|hit| !hit.before.is_empty() || !hit.after.is_empty());
    if had_context {
        let guarded = hits_data(&hits, &context_guard_message(&base_message), true, false);
        if data_fits(&guarded) {
            return crate::tool::outcome::ok_data(guarded);
        }
    }

    // Context was not enough (or absent): keep the largest whole-hit prefix
    // whose complete data object, including the actionable message, fits.
    let total = hits.len();
    let mut low = 0usize;
    let mut high = total;
    while low < high {
        let mid = low + (high - low).div_ceil(2);
        let candidate_message = hits_guard_message(&base_message, mid, total, had_context);
        let candidate = hits_data(&hits[..mid], &candidate_message, true, false);
        if data_fits(&candidate) {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    hits.truncate(low);
    let final_message = hits_guard_message(&base_message, low, total, had_context);
    let data = hits_data(&hits, &final_message, true, false);
    if data_fits(&data) {
        return crate::tool::outcome::ok_data(data);
    }

    // Defensive fallback for an implausibly tiny budget or huge fixed message:
    // never return an over-budget object. The wording stays actionable.
    crate::tool::outcome::ok_data(hits_data(
        &[],
        "Output capped at 8,000 cl100k tokens. Narrow pattern/path for fewer matches, or raise limit when more hits are needed.",
        true,
        false,
    ))
}

fn grep_virtual_file(
    raw_path: &str,
    options: &GrepOptions,
    execution: &ToolExecutionContext,
    examine_cap: usize,
) -> Result<ToolCallResult> {
    let stem = crate::session::transcript_file::try_parse_virtual_path(raw_path)
        .ok_or_else(|| LitecodeError::ToolExecution("invalid session transcript path".into()))?;
    let reader = execution.session_reader()?;
    let session_id = crate::engines::session_search::resolve_session_ref(reader, &stem)
        .map_err(|error| LitecodeError::ToolExecution(error.to_string()))?;
    let file = reader
        .transcript_file_blocking(&session_id)
        .map_err(|error| LitecodeError::ToolExecution(error.to_string()))?;
    let hits = grep_transcript(reader, &file, options, &execution.session_id)?;
    finish_virtual(hits, true, options, examine_cap)
}

fn grep_virtual_dir(
    options: &GrepOptions,
    execution: &ToolExecutionContext,
    examine_cap: usize,
) -> Result<ToolCallResult> {
    let reader = execution.session_reader()?;
    let listed = crate::session::transcript_file::list_virtual_paths(
        reader.list_session_ids_blocking().unwrap_or_default(),
    );
    let mut hits = Vec::new();
    for virtual_path in &listed {
        if !glob_allows(
            virtual_path,
            options.include.as_deref(),
            options.exclude.as_deref(),
        ) {
            continue;
        }
        let Some(stem) = crate::session::transcript_file::try_parse_virtual_path(virtual_path)
        else {
            continue;
        };
        let file = match reader.transcript_file_blocking(&stem) {
            Ok(file) => file,
            Err(error) => {
                tracing::warn!(path = %virtual_path, error = %error, "skip unread session transcript");
                continue;
            }
        };
        hits.extend(grep_transcript(
            reader,
            &file,
            options,
            &execution.session_id,
        )?);
    }
    let empty_message = if listed.is_empty() {
        "No files to search."
    } else if options.glob.is_some() && hits.is_empty() {
        "No files matched the glob filter."
    } else {
        "No matches found."
    };
    finish_virtual(hits, false, options, examine_cap).map(|result| {
        if result
            .data
            .as_ref()
            .and_then(|data| data.get("hits"))
            .and_then(Value::as_array)
            .is_some_and(|hits| hits.is_empty())
        {
            let mut result = result;
            if let Some(data) = result.data.as_mut() {
                data["message"] = serde_json::json!(empty_message);
            }
            result
        } else {
            result
        }
    })
}

fn finish_virtual(
    mut hits: Vec<GrepHit>,
    file_scoped: bool,
    options: &GrepOptions,
    examine_cap: usize,
) -> Result<ToolCallResult> {
    hits.sort_by(|a, b| {
        crate::workspace::glob_hit_key(&a.path)
            .cmp(&crate::workspace::glob_hit_key(&b.path))
            .then(a.line.cmp(&b.line))
    });
    if hits.is_empty() {
        let message = if file_scoped {
            "No matches found."
        } else {
            "No files to search."
        };
        return Ok(finish_hits(Vec::new(), message, false));
    }
    let truncated = hits.len() > options.limit
        || (examine_cap < options.limit + 1 && hits.len() >= examine_cap);
    if examine_cap < options.limit + 1 && hits.len() > examine_cap {
        hits.truncate(examine_cap);
    } else if hits.len() > options.limit {
        hits.truncate(options.limit);
    }
    let message = if truncated {
        limit_truncation_message(hits.len())
    } else {
        String::new()
    };
    Ok(finish_hits(hits, message, truncated))
}

fn grep_transcript(
    reader: &crate::session::SessionDataReader,
    file: &crate::session::transcript_file::TranscriptFile,
    options: &GrepOptions,
    caller_session_id: &str,
) -> Result<Vec<GrepHit>> {
    let source = if options.literal {
        regex::escape(&options.pattern)
    } else {
        options.pattern.clone()
    };
    let re = regex::RegexBuilder::new(&source)
        .case_insensitive(options.ignore_case)
        .build()
        .map_err(|error| {
            LitecodeError::ToolExecution(format!("invalid regular expression: {error}"))
        })?;
    let hidden = if !caller_session_id.is_empty() && caller_session_id == file.session_id {
        crate::engines::session_search::load_surface_seqs(reader, &file.session_id)
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let line_number = |index: usize| u32::try_from(index + 1).unwrap_or(u32::MAX);
    let visible: Vec<bool> = (1..=file.lines.len())
        .map(|line_no| {
            let line_no = u32::try_from(line_no).unwrap_or(u32::MAX);
            !file
                .seq_at(line_no)
                .is_some_and(|seq| hidden.contains(&seq))
        })
        .collect();
    let context = |start: usize, end: usize| -> Vec<GrepContextLine> {
        (start..end.min(file.lines.len()))
            .filter(|index| visible[*index])
            .map(|index| {
                let (text, _) = cap_match_line(&file.lines[index]);
                GrepContextLine {
                    line: line_number(index),
                    text,
                }
            })
            .collect()
    };
    let mut hits = Vec::new();
    for (index, line) in file.lines.iter().enumerate() {
        if !visible[index] {
            continue;
        }
        if re.is_match(line) {
            let (text, _) = cap_match_line(line);
            let before_start = index.saturating_sub(options.before);
            let after_end = index.saturating_add(1).saturating_add(options.after);
            hits.push(GrepHit {
                path: file.virtual_path.clone(),
                line: line_number(index),
                text,
                before: context(before_start, index),
                after: context(index.saturating_add(1), after_end),
            });
        }
    }
    Ok(hits)
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
            output_limit: usize::MAX,
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

    fn write(dir: &std::path::Path, rel: &str, body: &str) {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, body).unwrap();
    }

    fn write_many_files(dir: &std::path::Path, count: usize) {
        for file in 0..count {
            write(
                dir,
                &format!("deeply/nested/directory/number/{file:03}/source_file.rs"),
                "needle\n",
            );
        }
    }

    fn grep_data(result: &ToolCallResult) -> Value {
        result
            .data
            .clone()
            .expect("grep must return structured data")
    }

    fn hit_blob(result: &ToolCallResult) -> String {
        grep_data(result)["hits"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|hit| {
                format!(
                    "{} {}",
                    hit["path"].as_str().unwrap_or(""),
                    hit["text"].as_str().unwrap_or("")
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn limit_defaults_to_forty_and_notes_truncation_without_pagination() {
        let dir = tempfile::tempdir().unwrap();
        write_many_files(dir.path(), 45);
        let first = call_result(dir.path(), serde_json::json!({"pattern": "needle"}));
        assert_eq!(first.level, crate::types::ToolSignalLevel::Ok);
        assert!(first.content.is_empty(), "content must stay empty");
        let data = grep_data(&first);
        let hits = data["hits"].as_array().unwrap();
        assert_eq!(hits.len(), 40);
        assert_eq!(data["truncated"], true);
        let msg = data["message"].as_str().unwrap();
        assert!(msg.to_lowercase().contains("raise limit"), "{msg}");
        assert!(msg.to_lowercase().contains("narrow pattern/path"), "{msg}");
        assert!(!msg.to_lowercase().contains("next"), "{msg}");
        assert!(data.get("page").is_none());
        assert!(data.get("matches").is_none());
        assert!(data.get("cursor").is_none());
        assert!(data.get("next").is_none());
        assert!(data.get("offset").is_none());
        // Context folds into `text`, so hits stay exactly `{ path, line, text }`
        // whether or not context was requested: one shape, one table.
        for hit in hits {
            let hit = hit.as_object().unwrap();
            let keys: Vec<&str> = hit.keys().map(String::as_str).collect();
            assert_eq!(keys, ["path", "line", "text"]);
        }
    }

    #[test]
    fn before_after_context_folds_into_one_text_block() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "a.rs",
            "line one\nline two\nline three\nneedle\nline five\nline six\nline seven\n",
        );
        let result = call_result(
            dir.path(),
            serde_json::json!({"pattern": "needle", "before": 2, "after": 2}),
        );
        let data = grep_data(&result);
        assert!(data.get("truncated").is_none(), "{}", data["message"]);
        let hits = data["hits"].as_array().unwrap();
        assert_eq!(hits.len(), 1);
        let hit = &hits[0];
        assert_eq!(hit["path"], "a.rs");
        assert_eq!(hit["line"], 4);
        // One block, ripgrep notation, real file line numbers.
        assert_eq!(
            hit["text"],
            "2- line two\n3- line three\n4: needle\n5+ line five\n6+ line six"
        );
        let keys: Vec<&str> = hit.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(keys, ["path", "line", "text"]);
    }

    #[test]
    fn context_block_omits_missing_edges() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.rs", "line one\nline two\nneedle\nline four\n");
        // Match on the first line: no `-` rows.
        let result = call_result(
            dir.path(),
            serde_json::json!({"pattern": "line one", "before": 2, "after": 1}),
        );
        let data = grep_data(&result);
        let hit = &data["hits"].as_array().unwrap()[0];
        assert_eq!(hit["text"], "1: line one\n2+ line two");
        // Match on the last line: no `+` rows.
        let result = call_result(
            dir.path(),
            serde_json::json!({"pattern": "line four", "before": 1, "after": 2}),
        );
        let data = grep_data(&result);
        let hit = &data["hits"].as_array().unwrap()[0];
        assert_eq!(hit["text"], "3- needle\n4: line four");
        // No requested context: `text` stays the bare matching line.
        let result = call_result(
            dir.path(),
            serde_json::json!({"pattern": "line two", "before": 0, "after": 0}),
        );
        let data = grep_data(&result);
        let hit = &data["hits"].as_array().unwrap()[0];
        assert_eq!(hit["text"], "line two");
    }

    #[test]
    fn empty_context_line_keeps_marker_only() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.rs", "needle\n\nline three\n");
        let result = call_result(
            dir.path(),
            serde_json::json!({"pattern": "needle", "before": 0, "after": 2}),
        );
        let data = grep_data(&result);
        let hit = &data["hits"].as_array().unwrap()[0];
        // Blank source line: marker only, no trailing space in the cell.
        assert_eq!(hit["text"], "1: needle\n2+\n3+ line three");
    }

    #[test]
    fn context_parameters_are_bounded_and_written_to_the_contract() {
        let tool: std::sync::Arc<dyn crate::tool::Tool> = std::sync::Arc::new(GrepTool);
        let surface =
            crate::tool::code_contract::build_surface(&[tool]).expect("grep JS contract surface");
        let schema = &surface.tools["grep"].input_schema;
        assert_eq!(schema["properties"]["limit"]["default"], 40);
        assert_eq!(schema["properties"]["limit"]["maximum"], 100);
        assert_eq!(schema["properties"]["before"]["default"], 0);
        assert_eq!(schema["properties"]["before"]["maximum"], 5);
        assert_eq!(schema["properties"]["after"]["default"], 0);
        assert_eq!(schema["properties"]["after"]["maximum"], 5);
        assert!(schema["properties"].get("context").is_none());
        let data_schema = &surface.tools["grep"].output_schema;
        let hit_props = &data_schema["properties"]["hits"]["items"]["properties"];
        assert!(hit_props.get("before").is_none(), "{hit_props}");
        assert!(hit_props.get("after").is_none(), "{hit_props}");
        assert_eq!(hit_props["text"]["type"], "string");
        let options = parse_grep_options(&serde_json::json!({
            "pattern": "x", "before": 5, "after": 5
        }))
        .unwrap();
        assert_eq!((options.before, options.after), (5, 5));
        for name in ["before", "after"] {
            let error = parse_grep_options(&serde_json::json!({
                "pattern": "x", (name): 6
            }))
            .unwrap_err()
            .to_string();
            assert!(error.contains(name), "{error}");
            assert!(error.contains("between 0 and 5"), "{error}");
        }
    }

    #[test]
    fn token_guard_drops_context_before_keeping_all_hits() {
        let dir = tempfile::tempdir().unwrap();
        let filler = "错".repeat(500);
        let mut body = String::new();
        for index in 0..20 {
            for _ in 0..6 {
                body.push_str(&filler);
                body.push('\n');
            }
            body.push_str(&format!("needle {index}\n"));
        }
        write(dir.path(), "large.txt", &body);
        let result = call_result(
            dir.path(),
            serde_json::json!({"pattern": "needle", "before": 5, "after": 0}),
        );
        let data = grep_data(&result);
        assert_eq!(data["truncated"], true, "{}", data["message"]);
        let hits = data["hits"].as_array().unwrap();
        assert_eq!(
            hits.len(),
            20,
            "context reduction should fit without trimming hits"
        );
        for hit in hits {
            assert!(hit.get("before").is_none(), "{hit}");
            assert!(hit.get("after").is_none(), "{hit}");
        }
        let msg = data["message"].as_str().unwrap().to_lowercase();
        assert!(msg.contains("context was omitted"), "{msg}");
        assert!(msg.contains("8,000"), "{msg}");
        assert!(msg.contains("narrow pattern/path"), "{msg}");
        assert!(msg.contains("raise limit"), "{msg}");
        assert!(
            data_fits(&data),
            "{}",
            serde_json::to_string(&data).unwrap()
        );
    }

    #[test]
    fn token_guard_trims_hits_after_context_is_gone() {
        let dir = tempfile::tempdir().unwrap();
        let payload = "错".repeat(500);
        let mut body = String::new();
        for index in 0..100 {
            body.push_str(&format!("needle {index} {payload}\n"));
        }
        write(dir.path(), "dense.txt", &body);
        let result = call_result(
            dir.path(),
            serde_json::json!({"pattern": "needle", "limit": 100}),
        );
        let data = grep_data(&result);
        assert_eq!(data["truncated"], true, "{}", data["message"]);
        let hits = data["hits"].as_array().unwrap();
        assert!(!hits.is_empty());
        assert!(hits.len() < 100, "guard must trim: {}", hits.len());
        let msg = data["message"].as_str().unwrap();
        assert!(msg.contains("showing first"), "{msg}");
        assert!(msg.contains("of 100 hits"), "{msg}");
        assert!(msg.contains("8,000"), "{msg}");
        assert!(msg.to_lowercase().contains("narrow pattern/path"), "{msg}");
        assert!(msg.to_lowercase().contains("raise limit"), "{msg}");
        assert!(!msg.to_lowercase().contains("next"), "{msg}");
        assert!(data.get("cursor").is_none());
        assert!(data.get("next").is_none());
        assert!(data.get("offset").is_none());
        assert!(
            data_fits(&data),
            "{}",
            serde_json::to_string(&data).unwrap()
        );
    }

    #[test]
    fn virtual_transcript_grep_carries_exact_context() {
        let dir = tempfile::tempdir().unwrap();
        let reader = crate::session::SessionDataReader::open(
            &dir.path().join(".litecode").join("sessions.db"),
        );
        let lines = vec![
            "zero".to_string(),
            "one".to_string(),
            "two".to_string(),
            "needle".to_string(),
            "four".to_string(),
            "five".to_string(),
        ];
        let file = crate::session::transcript_file::TranscriptFile {
            session_id: "session-1".to_string(),
            virtual_path: crate::session::transcript_file::virtual_path_for("session-1"),
            line_index: (1..=lines.len())
                .map(|line| crate::session::transcript_file::LineSpan {
                    line: line as u32,
                    seq: 1,
                    kind: "item/user".to_string(),
                    item_type: "message".to_string(),
                    is_header: false,
                    char_start_in_item: 0,
                    char_end_in_item: 0,
                })
                .collect(),
            lines,
            tool_names: Default::default(),
            result_call_ids: Default::default(),
            session_read_calls: Default::default(),
        };
        let options = parse_grep_options(&serde_json::json!({
            "pattern": "needle", "before": 2, "after": 1
        }))
        .unwrap();
        let hits = grep_transcript(&reader, &file, &options, "").unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].line, 4);
        assert_eq!(
            hits[0]
                .before
                .iter()
                .map(|line| (line.line, line.text.as_str()))
                .collect::<Vec<_>>(),
            [(2, "one"), (3, "two")]
        );
        assert_eq!(
            hits[0]
                .after
                .iter()
                .map(|line| (line.line, line.text.as_str()))
                .collect::<Vec<_>>(),
            [(5, "four")]
        );
    }

    #[test]
    fn wide_search_never_writes_a_spill_file() {
        let dir = tempfile::tempdir().unwrap();
        write_many_files(dir.path(), 45);
        let result = call_result(dir.path(), serde_json::json!({"pattern": "needle"}));
        assert!(
            grep_data(&result)["message"]
                .as_str()
                .unwrap()
                .to_lowercase()
                .contains("raise limit")
        );
        let bash_dir = dir.path().join(".litecode").join("bash");
        if bash_dir.is_dir() {
            let spill = std::fs::read_dir(bash_dir)
                .unwrap()
                .filter_map(std::result::Result::ok)
                .any(|entry| entry.file_name().to_string_lossy().starts_with("grep_"));
            assert!(!spill, "grep must not spill matches to a file");
        }
    }

    fn assert_invalid_regex(pattern: &str, result: &ToolCallResult) {
        assert_eq!(result.error_code.as_deref(), Some("invalid_regex"));
        assert!(result.data.is_none());
        assert!(result.error.as_ref().is_some_and(|error| !error.retryable));
        let details = result.error_details.as_ref().expect("details");
        assert_eq!(details["pattern"], pattern);
        assert!(
            details["regex_error"]
                .as_str()
                .is_some_and(|text| !text.is_empty())
        );
        assert!(details.get("position").is_some());
        assert!(details.get("suggested_literal").is_none());
        assert!(details.get("suggested_pattern").is_none());
    }

    #[test]
    fn a_bare_brace_is_invalid_regex() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "a.rs",
            "pub struct ModelRequest {\n    model: String,\n}\n",
        );
        let pattern = "ModelRequest {";
        let result = call_result(dir.path(), serde_json::json!({"pattern": pattern}));
        assert_invalid_regex(pattern, &result);
        assert!(!result.content.contains("pub struct ModelRequest"));
        let retry = call_result(
            dir.path(),
            serde_json::json!({"pattern": pattern, "literal": true}),
        );
        assert!(retry.error.is_none(), "{}", retry.content);
        assert!(hit_blob(&retry).contains("ModelRequest"));
    }

    #[test]
    fn blank_glob_is_invalid_input() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.rs", "needle\n");
        let result = call_result(
            dir.path(),
            serde_json::json!({"pattern": "needle", "glob": "   "}),
        );
        assert_eq!(result.error_code.as_deref(), Some("invalid_input"));
        assert!(
            result
                .content
                .contains("parameter 'glob' must not be blank"),
            "{}",
            result.content
        );
        assert!(result.data.is_none());
    }
}

/// grep message states (merged from phase2_gate, WP-207).
#[cfg(test)]
#[allow(dead_code, unused_imports)]
mod message_contract_tests {
    use std::path::Path;

    use serde_json::{Value, json};

    use crate::tool::Tool;
    use crate::tool::trait_::ToolExecutionContext;
    use crate::tools::edit::EditTool;
    use crate::tools::glob::GlobTool;
    use crate::tools::grep::GrepTool;
    use crate::tools::read::ReadTool;
    use crate::types::ToolCallResult;

    fn execution(root: &Path, path_mode: crate::workspace::ToolPathMode) -> ToolExecutionContext {
        ToolExecutionContext {
            path_mode,
            workspace_root: root.to_path_buf(),
            call_id: String::new(),
            cancel: tokio_util::sync::CancellationToken::new(),
            output_limit: usize::MAX,
            session_id: String::new(),
            session: None,
        }
    }

    fn call<T: Tool>(
        tool: &T,
        root: &Path,
        input: Value,
        path_mode: crate::workspace::ToolPathMode,
    ) -> ToolCallResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(tool.execute(input, execution(root, path_mode)))
    }

    fn safe<T: Tool>(tool: &T, root: &Path, input: Value) -> ToolCallResult {
        call(tool, root, input, crate::workspace::ToolPathMode::Safe)
    }

    fn message(result: &ToolCallResult) -> String {
        result
            .data
            .as_ref()
            .and_then(|data| data.get("message"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    }

    fn message_of(data: &Value) -> String {
        data.get("message")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    }

    #[test]
    fn grep_message_glob_empty() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "needle\n").unwrap();
        let result = safe(
            &GrepTool,
            dir.path(),
            json!({"pattern": "needle", "glob": "*.md"}),
        );
        assert!(
            message(&result).to_lowercase().contains("glob"),
            "{}",
            message(&result)
        );
    }

    #[test]
    fn grep_message_path_excluded() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("hidden_dir")).unwrap();
        std::fs::write(dir.path().join("hidden_dir/hidden.rs"), "needle\n").unwrap();
        let root = dir.path().to_path_buf();
        let result = with_excluded_dir(|| {
            safe(
                &GrepTool,
                &root,
                json!({"pattern": "needle", "path": "hidden_dir"}),
            )
        });
        let msg = message(&result).to_lowercase();
        assert!(msg.contains("hidden") || msg.contains("filter"), "{msg}");
    }

    #[test]
    fn grep_message_binary_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("blob.bin"), [0u8, 1, 2, 3]).unwrap();
        let result = safe(
            &GrepTool,
            dir.path(),
            json!({"pattern": "a", "path": "blob.bin"}),
        );
        assert!(
            message(&result).to_lowercase().contains("binary"),
            "{}",
            message(&result)
        );
    }

    fn with_excluded_dir<R>(f: impl FnOnce() -> R) -> R {
        let mut file = crate::workspace::filter::WorkspaceExcludesFile::builtin_defaults();
        file.search_exclude.push("hidden_dir".into());
        let mut out = None;
        crate::workspace::filter::with_excludes_cache_for_test(file, || {
            out = Some(f());
        });
        out.expect("cache test body ran")
    }
}

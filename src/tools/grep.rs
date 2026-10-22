//! Agent `grep` tool.
//!
//! Bounded content search. Result shape is only `{ hits, message }`.
//! Hits are `{ path, line, text }` with 1-based line numbers. Truncation
//! and empty-result guidance live in `message` only — no cursor, page,
//! searched, base, or multi-enum reason fields.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

use crate::context_pipeline::Context;
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

const DEFAULT_GREP_LIMIT: usize = 20;
const MAX_GREP_LIMIT: usize = 100;
const MAX_MATCH_LINE_CHARS: usize = 500;
/// Backstop on matches examined while filling one call. Production asks for
/// at most `limit + 1`, so this fires only when a caller tightens it.
const GREP_EXAMINE_CAP: usize = 10_000;

#[derive(Debug, Clone, Serialize)]
struct GrepHit {
    path: String,
    line: u32,
    text: String,
}

struct GrepOptions {
    pattern: String,
    glob: Option<String>,
    include: Option<Vec<PathGlobMatcher>>,
    exclude: Option<Vec<PathGlobMatcher>>,
    ignore_case: bool,
    literal: bool,
    limit: usize,
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

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Regular expression over file contents, unless literal is true."
                },
                "path": {
                    "type": "string",
                    "description": "File or directory to search. Defaults to the workspace."
                },
                "glob": {
                    "type": "string",
                    "description": "Filter which files to search: one glob, or a comma-separated list. A leading ! excludes. Relative to path."
                },
                "literal": {
                    "type": "boolean",
                    "description": "Search for pattern as plain text. Default false."
                },
                "ignore_case": {
                    "type": "boolean",
                    "description": "Ignore case when matching. Default false."
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 100,
                    "description": "Maximum hits to return. Default 20, maximum 100."
                }
            },
            "required": ["pattern"],
            "additionalProperties": false
        })
    }

    fn data_schema(&self) -> Option<Value> {
        Some(serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "description": "Bounded content matches. Guidance lives only in message.",
            "properties": {
                "hits": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "additionalProperties": false,
                        "properties": {
                            "path": { "type": "string" },
                            "line": { "type": "integer", "minimum": 1 },
                            "text": { "type": "string" }
                        },
                        "required": ["path", "line", "text"]
                    }
                },
                "message": { "type": "string" }
            },
            "required": ["hits", "message"]
        }))
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

    fn description(&self, _ctx: &Context) -> String {
        crate::tools::description_text(include_str!("descriptions/grep.md"))
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
        ));
    }
    if !file_scoped
        && raw_path.is_some()
        && ignored_discovery_message(workspace, &resolved).is_some()
    {
        return Ok(finish_hits(
            Vec::new(),
            "Path is hidden by default search filters (.gitignore / workspace excludes).",
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
            ));
        }
        if scan.opened || !candidates.is_empty() {
            return Ok(finish_hits(Vec::new(), "No matches found."));
        }
        return Ok(finish_hits(Vec::new(), "No files to search."));
    }

    let message = if scan.truncated {
        format!(
            "Showing first {} hits; more matches exist. Raise limit to see more.",
            scan.hits.len()
        )
    } else if scan.line_capped {
        "One or more match lines were truncated in text.".into()
    } else {
        String::new()
    };
    Ok(finish_hits(scan.hits, message))
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
        before_context: 0,
        after_context: 0,
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
            GrepHit {
                path: candidate.display.clone(),
                line: hit.start_line,
                text,
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

fn finish_hits(hits: Vec<GrepHit>, message: impl Into<String>) -> ToolCallResult {
    // Code surface: facts in data; content empty (same as write/edit present).
    crate::tool::outcome::ok_data(serde_json::json!({
        "hits": hits,
        "message": message.into(),
    }))
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
        return Ok(finish_hits(Vec::new(), message));
    }
    let truncated = hits.len() > options.limit
        || (examine_cap < options.limit + 1 && hits.len() >= examine_cap);
    if examine_cap < options.limit + 1 && hits.len() > examine_cap {
        hits.truncate(examine_cap);
    } else if hits.len() > options.limit {
        hits.truncate(options.limit);
    }
    let message = if truncated {
        format!(
            "Showing first {} hits; more matches exist. Raise limit to see more.",
            hits.len()
        )
    } else {
        String::new()
    };
    Ok(finish_hits(hits, message))
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
    let mut hits = Vec::new();
    for (index, line) in file.lines.iter().enumerate() {
        let line_no = (index + 1) as u32;
        if file
            .seq_at(line_no)
            .is_some_and(|seq| hidden.contains(&seq))
        {
            continue;
        }
        if re.is_match(line) {
            let (text, _) = cap_match_line(line);
            hits.push(GrepHit {
                path: file.virtual_path.clone(),
                line: line_no,
                text,
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
    fn schema_exposes_simple_contract() {
        assert_eq!(GrepTool.timeout(), Some(Duration::from_secs(30)));
        assert!(GrepTool.is_cancellable());
        let schema = GrepTool.schema();
        let props = schema["properties"].as_object().unwrap();
        for expected in ["pattern", "path", "glob", "literal", "ignore_case", "limit"] {
            assert!(props.contains_key(expected), "missing {expected}");
        }
        for retired in ["cursor", "include_ignored", "case_sensitive"] {
            assert!(!props.contains_key(retired), "retired {retired} still in schema");
        }
        assert_eq!(schema["required"], serde_json::json!(["pattern"]));
        let data = GrepTool.data_schema().unwrap();
        let data_props = data["properties"].as_object().unwrap();
        assert!(data_props.contains_key("hits"));
        assert!(data_props.contains_key("message"));
        for retired in ["base", "matches", "page", "searched", "reason"] {
            assert!(!data_props.contains_key(retired), "retired {retired}");
        }
    }

    #[test]
    fn limit_bounds_hits_and_notes_truncation() {
        let dir = tempfile::tempdir().unwrap();
        write_many_files(dir.path(), 25);
        let first = call_result(dir.path(), serde_json::json!({"pattern": "needle"}));
        assert_eq!(first.level, crate::types::ToolSignalLevel::Ok);
        assert!(first.content.is_empty(), "content must stay empty");
        let data = grep_data(&first);
        assert_eq!(data["hits"].as_array().unwrap().len(), 20);
        assert!(
            data["message"].as_str().unwrap().contains("Raise limit"),
            "{}",
            data["message"]
        );
        assert!(data.get("page").is_none());
        assert!(data.get("matches").is_none());
    }

    #[test]
    fn retired_cursor_is_absent_from_schema() {
        let schema = GrepTool.schema();
        let props = schema["properties"].as_object().unwrap();
        assert!(!props.contains_key("cursor"));
        assert!(!props.contains_key("include_ignored"));
        // Direct execute ignores unknown keys; executor schema_validate rejects them.
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.rs", "needle\n");
        let result = call_result(
            dir.path(),
            serde_json::json!({"pattern": "needle", "cursor": "anything"}),
        );
        assert!(result.error.is_none(), "{}", result.content);
        assert_eq!(grep_data(&result)["hits"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn wide_search_never_writes_a_spill_file() {
        let dir = tempfile::tempdir().unwrap();
        write_many_files(dir.path(), 30);
        let result = call_result(dir.path(), serde_json::json!({"pattern": "needle"}));
        assert!(grep_data(&result)["message"].as_str().unwrap().contains("Raise limit"));
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
    fn invalid_limit_is_rejected() {
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
    fn match_text_is_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let long = format!("needle {}\n", "x".repeat(600));
        write(dir.path(), "long.rs", &long);
        let result = call_result(dir.path(), serde_json::json!({"pattern": "needle"}));
        let hit = &grep_data(&result)["hits"][0];
        assert!(hit["text"].as_str().unwrap().ends_with("..."));
        assert!(
            grep_data(&result)["message"]
                .as_str()
                .unwrap()
                .contains("truncated")
                || hit["text"].as_str().unwrap().ends_with("...")
        );
    }

    #[test]
    fn pattern_is_required_and_must_be_nonempty() {
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
    fn an_unclosed_class_is_not_searched_as_literal_text() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.rs", "let hit = found[0];\n");
        let pattern = "found[";
        let result = call_result(dir.path(), serde_json::json!({"pattern": pattern}));
        assert_invalid_regex(pattern, &result);
        assert!(!result.content.contains("found[0]"));
    }

    #[test]
    fn a_valid_regex_still_distinguishes_matches() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.rs", "needle_42 {\n");
        write(dir.path(), "b.rs", "needle_x {\nother_42 {\n");
        let result = call_result(dir.path(), serde_json::json!({"pattern": "needle_\\d+"}));
        let out = hit_blob(&result);
        assert!(out.contains("needle_42"), "got: {out}");
        assert!(
            !out.contains("needle_x") && !out.contains("other_42"),
            "got: {out}"
        );
    }

    #[test]
    fn empty_workspace_reports_no_files() {
        let dir = tempfile::tempdir().unwrap();
        let result = call_result(dir.path(), serde_json::json!({"pattern": "anything"}));
        let data = grep_data(&result);
        assert!(data["hits"].as_array().unwrap().is_empty());
        assert!(
            data["message"].as_str().unwrap().contains("No files"),
            "{}",
            data["message"]
        );
        assert!(data.get("reason").is_none());
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
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

    #[test]
    fn excluded_tree_stays_hidden_without_dialect_knob() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "hidden_dir/hidden.rs", "needle_lift\n");
        write(dir.path(), "visible.rs", "nothing\n");
        let path = dir.path().to_path_buf();
        let hidden =
            with_excluded_dir(|| call_result(&path, serde_json::json!({"pattern": "needle_lift"})));
        let hidden_data = grep_data(&hidden);
        assert!(hidden_data["hits"].as_array().unwrap().is_empty());
        assert!(
            !hit_blob(&hidden).contains("hidden.rs"),
            "must not lift excludes: {hidden_data}"
        );
        // include_ignored retired from schema; direct execute ignores unknown keys.
        assert!(!GrepTool.schema()["properties"]
            .as_object()
            .unwrap()
            .contains_key("include_ignored"));
        let ignored = with_excluded_dir(|| {
            call_result(
                &path,
                serde_json::json!({"pattern": "needle_lift", "include_ignored": true}),
            )
        });
        assert!(ignored.error.is_none(), "{}", ignored.content);
        assert!(
            grep_data(&ignored)["hits"].as_array().unwrap().is_empty(),
            "retired knob must not lift excludes"
        );
    }

    #[test]
    fn naming_an_excluded_tree_explains_in_message() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "hidden_dir/hidden.rs", "needle\n");
        let path = dir.path().to_path_buf();
        let result = with_excluded_dir(|| {
            call_result(
                &path,
                serde_json::json!({"pattern": "needle", "path": "hidden_dir"}),
            )
        });
        let data = grep_data(&result);
        assert!(data["hits"].as_array().unwrap().is_empty());
        assert!(
            data["message"].as_str().unwrap().to_lowercase().contains("hidden")
                || data["message"].as_str().unwrap().contains("filter"),
            "{}",
            data["message"]
        );
        assert!(data.get("reason").is_none());
    }

    #[test]
    fn multiple_hits_in_one_file_render_each_occurrence() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.rs", "needle\nfiller\nneedle\n");
        let result = call_result(dir.path(), serde_json::json!({"pattern": "needle"}));
        let data = grep_data(&result);
        assert_eq!(data["hits"].as_array().unwrap().len(), 2);
        assert_eq!(hit_blob(&result).matches("needle").count(), 2);
        assert_eq!(data["hits"][0]["line"], 1);
        assert_eq!(data["hits"][1]["line"], 3);
    }

    #[test]
    fn a_single_file_with_many_hits_still_bounds() {
        let dir = tempfile::tempdir().unwrap();
        let body: String = (0..40)
            .map(|i| format!("needle{i:02} {}\n", "y".repeat(400)))
            .collect();
        write(dir.path(), "wide.rs", &body);
        let result = call_result(dir.path(), serde_json::json!({"pattern": "needle"}));
        let data = grep_data(&result);
        assert_eq!(data["hits"].as_array().unwrap().len(), 20);
        assert!(data["message"].as_str().unwrap().contains("Raise limit"));
        assert!(hit_blob(&result).contains("needle00"));
    }

    #[test]
    fn unicode_matches_render_intact() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "cn.rs", "let s = \"世界\";\n");
        let result = call_result(dir.path(), serde_json::json!({"pattern": "世界"}));
        assert!(hit_blob(&result).contains("世界"));
    }

    #[test]
    fn ignore_case_finds_mixed_case() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.rs", "Needle\n");
        let sensitive = call_result(dir.path(), serde_json::json!({"pattern": "needle"}));
        assert!(grep_data(&sensitive)["hits"].as_array().unwrap().is_empty());
        let insensitive = call_result(
            dir.path(),
            serde_json::json!({"pattern": "needle", "ignore_case": true}),
        );
        assert_eq!(grep_data(&insensitive)["hits"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn a_path_outside_the_mode_is_reported_not_panicking() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.rs", "needle\n");
        let result = call_result_mode(
            dir.path(),
            serde_json::json!({"pattern": "needle", "path": dir.path().join("a.rs").display().to_string()}),
            crate::workspace::ToolPathMode::All,
        );
        assert!(
            hit_blob(&result).contains("a.rs"),
            "an absolute in-workspace path must work, got: {}",
            hit_blob(&result)
        );
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
        assert!(!prod.contains("stale_cursor"));
        assert!(!prod.contains("include_ignored"));
        assert!(!prod.contains("QueryCursor"));
        // LexicalQuery still has an internal case_sensitive field; public knob is ignore_case.
        let schema = GrepTool.schema();
        assert!(!schema["properties"].as_object().unwrap().contains_key("case_sensitive"));
        assert!(schema["properties"].as_object().unwrap().contains_key("ignore_case"));
    }

    #[test]
    fn virtual_session_grep_still_answers() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let sid = seed_session(root, "alpha\nVIRTUAL_GREP_NEEDLE here\ndelta");
        let path = crate::session::transcript_file::virtual_path_for(&sid);
        let result = call_result(
            root,
            serde_json::json!({"pattern": "VIRTUAL_GREP_NEEDLE", "path": path}),
        );
        assert!(hit_blob(&result).contains("VIRTUAL_GREP_NEEDLE"));
    }

    #[test]
    fn virtual_session_grep_miss_stays_a_plain_miss() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let sid = seed_session(root, "alpha\nbeta");
        let path = crate::session::transcript_file::virtual_path_for(&sid);
        let result = call_result(
            root,
            serde_json::json!({"pattern": "ABSENT_TOKEN", "path": path}),
        );
        let data = grep_data(&result);
        assert!(data["hits"].as_array().unwrap().is_empty());
        assert!(data["message"].as_str().unwrap().contains("No matches"));
        assert!(data.get("reason").is_none());
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
    fn data_schema_accepts_structured_hits() {
        let dir = tempfile::tempdir().unwrap();
        write_many_files(dir.path(), 3);
        let result = call_result(dir.path(), serde_json::json!({"pattern": "needle"}));
        let data = grep_data(&result);
        let schema = GrepTool.data_schema().expect("grep data schema");
        crate::tool::schema_validate(&schema, &data)
            .unwrap_or_else(|error| panic!("grep data does not match its schema: {error}: {data}"));
    }

    #[test]
    fn examine_cap_bounds_hits_in_message() {
        let dir = tempfile::tempdir().unwrap();
        write_many_files(dir.path(), 8);
        let result = dispatch(
            serde_json::json!({"pattern": "needle", "limit": 20}),
            execution(dir.path(), crate::workspace::ToolPathMode::Safe),
            3,
        );
        let data = grep_data(&result);
        assert_eq!(data["hits"].as_array().unwrap().len(), 3);
        assert!(data["message"].as_str().unwrap().contains("Raise limit"));
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
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
    fn grep_message_no_match() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello\n").unwrap();
        let result = safe(&GrepTool, dir.path(), json!({"pattern": "missing"}));
        assert!(message(&result).contains("No matches"), "{}", message(&result));
        assert!(result.data.unwrap()["hits"].as_array().unwrap().is_empty());
    }

    #[test]
    fn grep_message_corpus_empty() {
        let dir = tempfile::tempdir().unwrap();
        let result = safe(&GrepTool, dir.path(), json!({"pattern": "anything"}));
        assert!(message(&result).contains("No files"), "{}", message(&result));
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

    #[test]
    fn grep_literal_searches_metacharacters() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "a.b\naxb\n").unwrap();
        let result = safe(
            &GrepTool,
            dir.path(),
            json!({"pattern": "a.b", "literal": true}),
        );
        let data = result.data.unwrap();
        assert_eq!(data["hits"].as_array().unwrap().len(), 1);
        assert_eq!(data["hits"][0]["text"], "a.b");
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

//! Agent `grep` tool.
//!
//! One page of matches: `base`, `matches`, `page`, `searched`,
//! and `reason` when nothing matched. A cursor is the documented D-07
//! recommendation — query fingerprint plus a corpus version — not a stored copy
//! of the hits. D-07 itself stays undecided. Each call examines at most one page
//! of new matches, or `GREP_EXAMINE_CAP` when a test tightens that bound.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

use crate::context_pipeline::Context;
use crate::engines::code_search::{LexicalQuery, lexical_search_for_agent_cancellable};
use crate::tool::Tool;
use crate::tool::file_identity::{self, QueryCursor, encode_cursor, fingerprint};
use crate::tool::outcome::Page;
use crate::tool::trait_::ToolExecutionContext;
use crate::types::{LitecodeError, Result, ToolCallResult};
use crate::workspace::filter::{
    FilterPreset, PathGlobMatcher, WalkOptions, cheap_rel_under, compile_include_patterns,
    ignored_discovery_message, looks_binary,
    path_matches_include, private_knowledge_walker, split_glob_include_exclude, walk_builder_with,
};

const DEFAULT_GREP_LIMIT: usize = 20;
const MAX_GREP_LIMIT: usize = 100;
const MAX_MATCH_LINE_CHARS: usize = 500;
/// Backstop on matches examined while filling one page. Production pages ask
/// for at most `limit + 1` (101), so this fires only when a caller tightens it.
const GREP_EXAMINE_CAP: usize = 10_000;

#[derive(Debug, Clone, Serialize)]
struct GrepMatchData {
    path: String,
    start_line: u32,
    end_line: u32,
    line_text: String,
    line_truncated: bool,
}

struct GrepOptions {
    pattern: String,
    glob: Option<String>,
    include: Option<Vec<PathGlobMatcher>>,
    exclude: Option<Vec<PathGlobMatcher>>,
    case_sensitive: bool,
    include_ignored: bool,
    literal: bool,
    cursor: Option<String>,
    limit: usize,
}

struct Candidate {
    abs: PathBuf,
    display: String,
    len: u64,
    mtime: u64,
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
                    "description": "File or directory to search. Defaults to the workspace. Outside the workspace, returned paths are absolute."
                },
                "glob": {
                    "type": "string",
                    "description": "Filter which files to search: one glob, or a comma-separated list. A leading ! excludes. Relative to path."
                },
                "include_ignored": {
                    "type": "boolean",
                    "description": "Also search gitignored and workspace-excluded paths. Default false."
                },
                "case_sensitive": {
                    "type": "boolean",
                    "description": "Default true. Set false to ignore case."
                },
                "literal": {
                    "type": "boolean",
                    "description": "Search for pattern as plain text. Default false."
                },
                "cursor": {
                    "type": "string",
                    "description": "Continuation from page.next. Bound to pattern, path, glob, filters, and the corpus version. A different query rejects stale_cursor."
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 100,
                    "description": "Maximum matches in this page. Default 20, maximum 100."
                }
            },
            "required": ["pattern"],
            "additionalProperties": false
        })
    }

    fn data_schema(&self) -> Option<Value> {
        Some(serde_json::json!({
            "type": "object",
            "description": "One page of content matches.",
            "properties": {
                "base": {
                    "type": "string",
                    "enum": ["workspace", "absolute"]
                },
                "matches": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "path": { "type": "string" },
                            "start_line": { "type": "integer", "minimum": 1 },
                            "end_line": { "type": "integer", "minimum": 1 },
                            "line_text": { "type": "string" },
                            "line_truncated": { "type": "boolean" }
                        },
                        "required": ["path", "start_line", "end_line", "line_text", "line_truncated"]
                    }
                },
                "page": {
                    "type": "object",
                    "properties": {
                        "has_more": { "type": "boolean" },
                        "next": { "type": ["string", "null"] },
                        "total": { "type": ["integer", "null"], "minimum": 0 }
                    },
                    "required": ["has_more", "next"]
                },
                "searched": {
                    "type": "object",
                    "properties": {
                        "files": { "type": "integer", "minimum": 0 },
                        "excluded": {
                            "type": "boolean",
                            "description": "True when default exclusion filters were active, or the path itself was excluded."
                        }
                    },
                    "required": ["files", "excluded"]
                },
                "reason": {
                    "type": "string",
                    "enum": ["no_match", "corpus_empty", "glob_empty", "path_excluded", "binary_file", "file_not_searched"]
                }
            },
            "required": ["base", "matches", "page", "searched"]
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
            match tokio::task::spawn_blocking(move || {
                dispatch(input, execution, GREP_EXAMINE_CAP)
            })
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
        parse_grep_options(input).map(|_| ()).map_err(|error| match error {
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
    let case_sensitive = optional_bool(input, "case_sensitive")?.unwrap_or(true);
    let include_ignored = optional_bool(input, "include_ignored")?.unwrap_or(false);
    let literal = optional_bool(input, "literal")?.unwrap_or(false);
    let glob = optional_string(input, "glob")?;
    let (include, exclude) = match glob.as_deref() {
        Some(raw) => {
            let (include, exclude) = split_glob_include_exclude(raw);
            (
                compile_glob_list(include)?,
                compile_glob_list(exclude)?,
            )
        }
        None => (None, None),
    };
    let cursor = match input.get("cursor") {
        None | Some(Value::Null) => None,
        Some(Value::String(value)) => Some(value.clone()),
        Some(_) => {
            return Err(LitecodeError::ToolExecution(
                "parameter 'cursor' expected a string".into(),
            ));
        }
    };
    Ok(GrepOptions {
        pattern,
        glob,
        include,
        exclude,
        case_sensitive,
        include_ignored,
        literal,
        cursor,
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
    let case_sensitive = match input.get("case_sensitive") {
        None | Some(Value::Null) => true,
        Some(Value::Bool(value)) => *value,
        Some(_) => return None,
    };
    match regex::RegexBuilder::new(pattern)
        .case_insensitive(!case_sensitive)
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

fn query_fingerprint(options: &GrepOptions, path: &str) -> String {
    fingerprint(&[
        ("pattern", &options.pattern),
        ("path", path),
        ("glob", options.glob.as_deref().unwrap_or("")),
        (
            "case_sensitive",
            if options.case_sensitive { "true" } else { "false" },
        ),
        (
            "include_ignored",
            if options.include_ignored { "true" } else { "false" },
        ),
        ("literal", if options.literal { "true" } else { "false" }),
    ])
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
    let base = if outside { "absolute" } else { "workspace" };
    let file_scoped = resolved.is_file();
    if !file_scoped && !resolved.is_dir() {
        return Ok(crate::tools::file_path::coded_error(
            "not_found",
            format!("path does not exist: {}", resolved.display()),
        ));
    }
    if file_scoped && looks_binary(&resolved) {
        return Ok(empty_result(base, 0, !options.include_ignored, "binary_file"));
    }
    if !file_scoped
        && raw_path.is_some()
        && ignored_discovery_message(workspace, &resolved).is_some()
    {
        return Ok(empty_result(base, 0, true, "path_excluded"));
    }

    let preset = if options.include_ignored {
        FilterPreset::NoIgnore
    } else {
        FilterPreset::Search
    };
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
    let version = file_identity::version_hash(
        candidates
            .iter()
            .map(|candidate| format!("{}\n{}\n{}", candidate.display, candidate.len, candidate.mtime)),
    );
    let fp = query_fingerprint(options, raw_path.unwrap_or(""));
    let bound = match file_identity::bind_cursor(options.cursor.as_deref(), &fp, &version) {
        Ok(bound) => bound,
        Err(stale) => return Ok(stale),
    };
    if let Some(path) = bound.as_ref().and_then(|cursor| cursor.resume_path.as_ref())
        && !candidates.iter().any(|candidate| candidate.display == *path)
    {
        return Ok(file_identity::stale_cursor_result("offset"));
    }

    if options.glob.is_some() && candidates.is_empty() {
        return Ok(empty_result(base, 0, !options.include_ignored, "glob_empty"));
    }

    let scan = collect_matches(
        workspace,
        outside,
        &candidates,
        options,
        preset,
        bound.as_ref(),
        examine_cap,
        &execution.cancel,
    )?;
    if scan.matches.is_empty() && !scan.has_more {
        if file_scoped && !scan.opened {
            return Ok(empty_result(
                base,
                0,
                !options.include_ignored,
                "file_not_searched",
            ));
        }
        if scan.opened || !candidates.is_empty() {
            return Ok(empty_result(
                base,
                candidates.len() as u64,
                !options.include_ignored,
                "no_match",
            ));
        }
        return Ok(empty_result(
            base,
            0,
            !options.include_ignored,
            "corpus_empty",
        ));
    }

    Ok(page_result(
        &scan.matches,
        base,
        scan.has_more,
        &fp,
        &version,
        candidates.len() as u64,
        !options.include_ignored,
        None,
    ))
}

struct Scan {
    matches: Vec<GrepMatchData>,
    has_more: bool,
    opened: bool,
}

fn collect_matches(
    workspace: &Path,
    outside: bool,
    candidates: &[Candidate],
    options: &GrepOptions,
    preset: FilterPreset,
    cursor: Option<&QueryCursor>,
    examine_cap: usize,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<Scan> {
    let limit = options.limit;
    let target = (limit + 1).min(examine_cap.max(1));
    let resume_path = cursor.and_then(|cursor| cursor.resume_path.clone());
    let resume_line = cursor.and_then(|cursor| cursor.resume_line).unwrap_or(0);
    let mut kept = Vec::new();
    let mut opened = false;
    let mut skipping = resume_path.is_some();
    let mut last_file_full = false;
    let mut files_left = false;
    for (index, candidate) in candidates.iter().enumerate() {
        if cancel.is_cancelled() {
            return Err(LitecodeError::ToolExecution("search cancelled".into()));
        }
        if skipping {
            if resume_path.as_deref() != Some(candidate.display.as_str()) {
                continue;
            }
        }
        let floor = if skipping { resume_line } else { 0 };
        skipping = false;
        if kept.len() >= target {
            files_left = true;
            break;
        }
        let fetch = target - kept.len();
        let (hits, file_opened) = search_file(
            workspace,
            outside,
            candidate,
            options,
            preset,
            floor,
            fetch,
            cancel,
        )?;
        if file_opened {
            opened = true;
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
    let has_more = has_extra || (hit_cap && (last_file_full || files_left));
    Ok(Scan {
        matches: kept,
        has_more,
        opened,
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
) -> Result<(Vec<GrepMatchData>, bool)> {
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
        case_sensitive: options.case_sensitive,
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
    let outcome =
        lexical_search_for_agent_cancellable(&query, preset, workspace, Some(cancel))?;
    let opened = outcome.files_searched > 0 || !outcome.matches.is_empty();
    let matches = outcome
        .matches
        .into_iter()
        .map(|hit| {
            let (line_text, line_truncated) = cap_match_line(&hit.line_text);
            GrepMatchData {
                path: candidate.display.clone(),
                start_line: hit.start_line,
                end_line: hit.end_line,
                line_text,
                line_truncated,
            }
        })
        .collect();
    Ok((matches, opened))
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
            crate::workspace::glob_hit_key(&a.display).cmp(&crate::workspace::glob_hit_key(&b.display))
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
    if !glob_allows(&display, options.include.as_deref(), options.exclude.as_deref()) {
        return;
    }
    if !seen.insert(display.clone()) {
        return;
    }
    let (len, mtime) = file_stamp(abs);
    found.push(Candidate {
        abs: abs.to_path_buf(),
        display,
        len,
        mtime,
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

fn file_stamp(path: &Path) -> (u64, u64) {
    let Ok(meta) = std::fs::metadata(path) else {
        return (0, 0);
    };
    let mtime = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    (meta.len(), mtime)
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

fn page_result(
    matches: &[GrepMatchData],
    base: &str,
    has_more: bool,
    fp: &str,
    version: &str,
    files: u64,
    excluded: bool,
    reason: Option<&str>,
) -> ToolCallResult {
    let next = has_more.then(|| {
        matches.last().map(|hit| {
            encode_cursor(&QueryCursor {
                fp: fp.to_string(),
                ver: version.to_string(),
                offset: 0,
                resume_path: Some(hit.path.clone()),
                resume_line: Some(u64::from(hit.start_line)),
            })
        })
    });
    let next = next.flatten();
    let has_more = next.is_some();
    finish_result(base, matches.to_vec(), has_more, next, files, excluded, reason)
}

fn empty_result(base: &str, files: u64, excluded: bool, reason: &str) -> ToolCallResult {
    finish_result(base, Vec::new(), false, None, files, excluded, Some(reason))
}

fn finish_result(
    base: &str,
    matches: Vec<GrepMatchData>,
    has_more: bool,
    next: Option<String>,
    files: u64,
    excluded: bool,
    reason: Option<&str>,
) -> ToolCallResult {
    let page = Page {
        has_more,
        next,
        total: None,
    };
    let mut data = serde_json::json!({
        "base": base,
        "matches": matches,
        "page": &page,
        "searched": {"files": files, "excluded": excluded},
    });
    if let Some(reason) = reason {
        data["reason"] = serde_json::json!(reason);
    }
    crate::tool::outcome::ok_data(data)
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
    let matches = grep_transcript(reader, &file, options, &execution.session_id)?;
    finish_virtual(raw_path, matches, true, options, execution, examine_cap)
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
    let mut matches = Vec::new();
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
        matches.extend(grep_transcript(
            reader,
            &file,
            options,
            &execution.session_id,
        )?);
    }
    let empty_reason = if listed.is_empty() {
        "corpus_empty"
    } else if options.glob.is_some() && matches.is_empty() {
        "glob_empty"
    } else {
        "no_match"
    };
    finish_virtual(
        ".litecode/sessions",
        matches,
        false,
        options,
        execution,
        examine_cap,
    )
    .map(|result| {
        if result
            .data
            .as_ref()
            .and_then(|data| data.get("matches"))
            .and_then(Value::as_array)
            .is_some_and(|matches| matches.is_empty())
        {
            let mut result = result;
            if let Some(data) = result.data.as_mut() {
                data["reason"] = serde_json::json!(empty_reason);
            }
            result
        } else {
            result
        }
    })
}

fn finish_virtual(
    path_key: &str,
    mut matches: Vec<GrepMatchData>,
    file_scoped: bool,
    options: &GrepOptions,
    _execution: &ToolExecutionContext,
    examine_cap: usize,
) -> Result<ToolCallResult> {
    matches.sort_by(|a, b| {
        crate::workspace::glob_hit_key(&a.path)
            .cmp(&crate::workspace::glob_hit_key(&b.path))
            .then(a.start_line.cmp(&b.start_line))
    });
    let version = file_identity::version_hash(matches.iter().map(|hit| {
        format!("{}:{}:{}", hit.path, hit.start_line, hit.line_text)
    }));
    let fp = query_fingerprint(options, path_key);
    let bound = match file_identity::bind_cursor(options.cursor.as_deref(), &fp, &version) {
        Ok(bound) => bound,
        Err(stale) => return Ok(stale),
    };
    if matches.is_empty() {
        let reason = if file_scoped { "no_match" } else { "corpus_empty" };
        return Ok(empty_result(
            "workspace",
            u64::from(file_scoped),
            !options.include_ignored,
            reason,
        ));
    }
    let resume_path = bound.as_ref().and_then(|cursor| cursor.resume_path.clone());
    let resume_line = bound.and_then(|cursor| cursor.resume_line).unwrap_or(0);
    let target = (options.limit + 1).min(examine_cap.max(1));
    let mut page = Vec::new();
    let mut started = resume_path.is_none();
    for hit in matches {
        if !started {
            if resume_path.as_deref() != Some(hit.path.as_str()) {
                continue;
            }
            if u64::from(hit.start_line) <= resume_line {
                continue;
            }
            started = true;
        }
        page.push(hit);
        if page.len() >= target {
            break;
        }
    }
    let hit_cap = examine_cap < options.limit + 1 && page.len() >= examine_cap;
    let has_extra = page.len() > options.limit;
    if has_extra {
        page.truncate(options.limit);
    }
    Ok(page_result(
        &page,
        "workspace",
        has_extra || hit_cap,
        &fp,
        &version,
        1,
        !options.include_ignored,
        None,
    ))
}

fn grep_transcript(
    reader: &crate::session::SessionDataReader,
    file: &crate::session::transcript_file::TranscriptFile,
    options: &GrepOptions,
    caller_session_id: &str,
) -> Result<Vec<GrepMatchData>> {
    let source = if options.literal {
        regex::escape(&options.pattern)
    } else {
        options.pattern.clone()
    };
    let re = regex::RegexBuilder::new(&source)
        .case_insensitive(!options.case_sensitive)
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
    let mut matches = Vec::new();
    for (index, line) in file.lines.iter().enumerate() {
        let line_no = (index + 1) as u32;
        if file
            .seq_at(line_no)
            .is_some_and(|seq| hidden.contains(&seq))
        {
            continue;
        }
        if re.is_match(line) {
            let (line_text, line_truncated) = cap_match_line(line);
            matches.push(GrepMatchData {
                path: file.virtual_path.clone(),
                start_line: line_no,
                end_line: line_no,
                line_text,
                line_truncated,
            });
        }
    }
    Ok(matches)
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

    fn match_blob(result: &ToolCallResult) -> String {
        grep_data(result)["matches"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|hit| {
                format!(
                    "{} {}",
                    hit["path"].as_str().unwrap_or(""),
                    hit["line_text"].as_str().unwrap_or("")
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn schema_exposes_pagination_and_literal() {
        assert_eq!(GrepTool.timeout(), Some(Duration::from_secs(30)));
        assert!(GrepTool.is_cancellable());
        let schema = GrepTool.schema();
        let props = schema["properties"].as_object().unwrap();
        for expected in [
            "case_sensitive",
            "cursor",
            "glob",
            "include_ignored",
            "limit",
            "literal",
            "path",
            "pattern",
        ] {
            assert!(props.contains_key(expected), "missing {expected}");
        }
        assert_eq!(schema["required"], serde_json::json!(["pattern"]));
        assert!(props["literal"]["description"]
            .as_str()
            .unwrap()
            .contains("plain text"));
    }

    #[test]
    fn page_is_bounded_and_cursor_continues() {
        let dir = tempfile::tempdir().unwrap();
        write_many_files(dir.path(), 25);

        let first = call_result(dir.path(), serde_json::json!({"pattern": "needle"}));
        assert_eq!(first.level, crate::types::ToolSignalLevel::Ok);
        let first_data = grep_data(&first);
        assert_eq!(first_data["matches"].as_array().unwrap().len(), 20);
        assert_eq!(first_data["page"]["has_more"], true);
        let cursor = first_data["page"]["next"].as_str().unwrap().to_string();

        let second = call_result(
            dir.path(),
            serde_json::json!({"pattern": "needle", "cursor": cursor}),
        );
        let second_data = grep_data(&second);
        assert_eq!(second_data["matches"].as_array().unwrap().len(), 5);
        assert_eq!(second_data["page"]["has_more"], false);
        assert!(second_data["page"]["next"].is_null());

        let mut seen = HashSet::new();
        for data in [&first_data, &second_data] {
            for hit in data["matches"].as_array().unwrap() {
                let key = format!(
                    "{}:{}",
                    hit["path"].as_str().unwrap(),
                    hit["start_line"].as_u64().unwrap()
                );
                assert!(seen.insert(key), "pages overlapped");
            }
        }
        assert_eq!(seen.len(), 25);
    }

    #[test]
    fn changed_pattern_reuses_cursor_as_stale() {
        let dir = tempfile::tempdir().unwrap();
        write_many_files(dir.path(), 25);
        let first = call_result(dir.path(), serde_json::json!({"pattern": "needle"}));
        let cursor = grep_data(&first)["page"]["next"]
            .as_str()
            .unwrap()
            .to_string();
        let stale = call_result(
            dir.path(),
            serde_json::json!({"pattern": "other", "cursor": cursor}),
        );
        assert_eq!(stale.error_code.as_deref(), Some("stale_cursor"));
        assert_eq!(
            stale.error_details.as_ref().unwrap()["reason"],
            "query"
        );
    }

    #[test]
    fn wide_search_never_writes_a_spill_file() {
        let dir = tempfile::tempdir().unwrap();
        write_many_files(dir.path(), 30);
        let result = call_result(dir.path(), serde_json::json!({"pattern": "needle"}));
        assert_eq!(grep_data(&result)["page"]["has_more"], true);

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
        assert!(details["regex_error"].as_str().is_some_and(|text| !text.is_empty()));
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
        assert!(match_blob(&retry).contains("ModelRequest"));
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
        let out = match_blob(&result);
        assert!(out.contains("needle_42"), "got: {out}");
        assert!(
            !out.contains("needle_x") && !out.contains("other_42"),
            "got: {out}"
        );
    }

    #[test]
    fn empty_workspace_reports_the_corpus_without_naming_a_knob() {
        let dir = tempfile::tempdir().unwrap();
        let result = call_result(dir.path(), serde_json::json!({"pattern": "anything"}));
        let data = grep_data(&result);
        assert_eq!(data["reason"], "corpus_empty");
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(!result.content.contains("-u"), "{}", result.content);
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
    fn excluded_tree_requires_explicit_include_ignored() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "hidden_dir/hidden.rs", "needle_lift\n");
        write(dir.path(), "visible.rs", "nothing\n");
        let path = dir.path().to_path_buf();

        let hidden = with_excluded_dir(|| {
            call_result(&path, serde_json::json!({"pattern": "needle_lift"}))
        });
        let hidden_data = grep_data(&hidden);
        assert_eq!(hidden_data["reason"], "no_match");
        assert_eq!(hidden_data["searched"]["excluded"], true);
        assert!(
            !match_blob(&hidden).contains("hidden.rs"),
            "zero-hit must not lift excludes implicitly: {hidden_data}"
        );

        let included = with_excluded_dir(|| {
            call_result(
                &path,
                serde_json::json!({"pattern": "needle_lift", "include_ignored": true}),
            )
        });
        let included_blob = match_blob(&included);
        assert!(included_blob.contains("hidden.rs"), "got: {included_blob}");
        assert_eq!(grep_data(&included)["searched"]["excluded"], false);
    }

    #[test]
    fn naming_an_excluded_tree_is_refused_with_the_reason() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "hidden_dir/hidden.rs", "needle\n");
        let path = dir.path().to_path_buf();
        let result = with_excluded_dir(|| {
            call_result(
                &path,
                serde_json::json!({"pattern": "needle", "path": "hidden_dir"}),
            )
        });
        assert_eq!(grep_data(&result)["reason"], "path_excluded");
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(grep_data(&result)["matches"].as_array().unwrap().is_empty());
    }

    #[test]
    fn include_ignored_miss_names_the_expanded_corpus() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "hidden_dir/hidden.rs", "present\n");
        let path = dir.path().to_path_buf();
        let result = with_excluded_dir(|| {
            call_result(
                &path,
                serde_json::json!({"pattern": "absent_everywhere", "include_ignored": true}),
            )
        });
        let data = grep_data(&result);
        assert_eq!(data["matches"].as_array().unwrap().len(), 0);
        assert_eq!(data["searched"]["excluded"], false);
        assert!(
            result.diagnostics.is_empty(),
            "{:?}",
            result.diagnostics
        );
    }

    #[test]
    fn multiple_hits_in_one_file_render_each_occurrence() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.rs", "needle\nfiller\nneedle\n");
        let result = call_result(dir.path(), serde_json::json!({"pattern": "needle"}));
        let data = grep_data(&result);
        assert_eq!(data["matches"].as_array().unwrap().len(), 2);
        assert_eq!(match_blob(&result).matches("needle").count(), 2);
    }

    #[test]
    fn long_match_lines_are_marked_truncated() {
        let dir = tempfile::tempdir().unwrap();
        let long = format!("needle {}\n", "x".repeat(600));
        write(dir.path(), "big.rs", &long);
        let data = grep_data(&call_result(dir.path(), serde_json::json!({"pattern": "needle"})));
        assert_eq!(data["matches"][0]["line_truncated"], true);
    }

    #[test]
    fn a_single_file_with_many_hits_still_returns_a_page() {
        let dir = tempfile::tempdir().unwrap();
        let body: String = (0..40)
            .map(|i| format!("needle{i:02} {}\n", "y".repeat(400)))
            .collect();
        write(dir.path(), "wide.rs", &body);
        let result = call_result(dir.path(), serde_json::json!({"pattern": "needle"}));
        let data = grep_data(&result);
        assert_eq!(data["matches"].as_array().unwrap().len(), 20);
        assert_eq!(data["page"]["has_more"], true);
        assert!(match_blob(&result).contains("needle00"));
    }

    #[test]
    fn unicode_matches_render_intact() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "cn.rs", "let s = \"世界\";\n");
        let result = call_result(dir.path(), serde_json::json!({"pattern": "世界"}));
        assert!(match_blob(&result).contains("世界"));
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
            match_blob(&result).contains("a.rs"),
            "an absolute in-workspace path must work, got: {}",
            match_blob(&result)
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
            result.content.contains("parameter 'glob' must not be blank"),
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
        assert!(match_blob(&result).contains("VIRTUAL_GREP_NEEDLE"));
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
        assert_eq!(data["reason"], "no_match");
        assert!(data["matches"].as_array().unwrap().is_empty());
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

    #[test]
    fn phase2_grep_scan_budget_is_visible() {
        let dir = tempfile::tempdir().unwrap();
        write_many_files(dir.path(), 8);
        let result = dispatch(
            serde_json::json!({"pattern": "needle", "limit": 20}),
            execution(dir.path(), crate::workspace::ToolPathMode::Safe),
            3,
        );
        let data = grep_data(&result);
        assert_eq!(data["matches"].as_array().unwrap().len(), 3);
        assert_eq!(data["page"]["has_more"], true);
        assert!(data["page"]["next"].is_string());
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    }
}

use std::time::Duration;

use serde_json::Value;

use crate::context_pipeline::Context;
use crate::tool::Tool;
use crate::tool::file_identity::{self, QueryCursor, encode_cursor, fingerprint};
use crate::tool::outcome::Page;
use crate::tool::trait_::ToolExecutionContext;
use crate::types::{Result, ToolCallResult};
use crate::workspace::filter::{
    FilterPreset, RelPathCtx, WalkOptions, cheap_rel_under, compile_include_pattern,
    normalize_pattern, path_matches_include, private_knowledge_walker, walk_builder_with,
};

const DEFAULT_GLOB_LIMIT: usize = 1000;
const MAX_GLOB_LIMIT: usize = 5000;

pub struct GlobTool;

impl Tool for GlobTool {
    fn name(&self) -> &str {
        "glob"
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
                    "description": "Glob relative to path (or workspace root if path omitted). Example: '**/*.rs'. The pattern is used as given; a repeated path prefix is not removed. If path is 'src', use '**/*.rs', not 'src/**/*.rs'."
                },
                "path": {
                    "type": "string",
                    "description": "Optional directory to search under (workspace-relative preferred; absolute paths outside the workspace only under All permission). Pattern is matched relative to this directory. Session transcripts are listed only when this is `.litecode/sessions`."
                },
                "include_ignored": {
                    "type": "boolean",
                    "description": "Also search gitignored and workspace-excluded paths. Default false."
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum paths in this page. Default 1000, maximum 5000."
                },
                "cursor": {
                    "type": "string",
                    "description": "Opaque continuation from a previous glob page. Bound to pattern, path, and filters. A different query rejects stale_cursor."
                }
            },
            "required": ["pattern"],
            "additionalProperties": false
        })
    }

    fn data_schema(&self) -> Option<Value> {
        Some(serde_json::json!({
            "type": "object",
            "description": "One page of matching paths.",
            "properties": {
                "base": {
                    "type": "string",
                    "enum": ["workspace", "absolute"],
                    "description": "workspace: paths are workspace-relative. absolute: paths are absolute."
                },
                "paths": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Matching paths sorted by depth, then parent directory, then name."
                },
                "page": {
                    "type": "object",
                    "properties": {
                        "has_more": { "type": "boolean" },
                        "next": { "type": ["string", "null"] },
                        "total": { "type": "integer", "minimum": 0 }
                    },
                    "required": ["has_more", "next"]
                },
                "reason": {
                    "type": "string",
                    "enum": ["no_match", "path_excluded"],
                    "description": "no_match: nothing matched. path_excluded: the path is hidden by the default filters."
                }
            },
            "required": ["base", "paths", "page"]
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
            match tokio::task::spawn_blocking(move || GlobTool.call_for_execution(input, execution))
                .await
            {
                Ok(result) => result,
                Err(error) => ToolCallResult::error_with_kind(
                    crate::types::ToolErrorKind::Internal,
                    format!("glob worker failed: {error}"),
                    false,
                )
                .with_error_code("internal"),
            }
        })
    }

    fn call_inner(&self, input: Value) -> ToolCallResult {
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
        crate::tools::description_text(include_str!("descriptions/glob.md"))
    }

    fn timeout(&self) -> Option<Duration> {
        Some(Duration::from_secs(30))
    }

    fn is_cancellable(&self) -> bool {
        true
    }
}

impl GlobTool {
    fn call_for_execution(&self, input: Value, execution: ToolExecutionContext) -> ToolCallResult {
        let pattern = match crate::tool::require_nonempty_string(&input, "pattern") {
            Ok(s) => s,
            Err(e) => return coded("invalid_input", e),
        };
        if pattern_reaches_parent(pattern) {
            return coded(
                "invalid_input",
                format!("pattern '{pattern}' reaches outside the search directory"),
            );
        }

        let no_ignore = input
            .get("include_ignored")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let path_arg = input["path"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty());

        let limit = match parse_glob_limit(&input) {
            Ok(limit) => limit,
            Err(error) => return coded("invalid_input", error),
        };
        let outside = path_arg.is_some_and(|path| {
            crate::workspace::resolve_agent(&execution.workspace_root, path, execution.path_mode)
                .is_ok_and(|resolved| {
                    !crate::config::path::is_under(&resolved, &execution.workspace_root)
                })
        });

        if let Some(path) = path_arg
            && crate::session::transcript_file::is_virtual_session_dir(path)
        {
            let results = match glob_virtual_sessions(&execution, pattern) {
                Ok(r) => r,
                Err(e) => return crate::tools::file_path::litecode_error(e),
            };
            return finish_glob(
                &results.hits,
                file_identity::PathBase::Workspace,
                pattern,
                path_arg,
                no_ignore,
                limit,
                input.get("cursor").and_then(Value::as_str),
                false,
            );
        }

        let search_path = match path_arg {
            Some(path) => {
                match crate::workspace::resolve_agent(
                    &execution.workspace_root,
                    path,
                    execution.path_mode,
                ) {
                    Ok(path) => path,
                    Err(error) => return crate::tools::file_path::tool_path_error(error),
                }
            }
            None => execution.workspace_root.clone(),
        };

        if !search_path.exists() {
            return coded(
                "not_found",
                format!("path does not exist: {}", search_path.display()),
            );
        }
        if !search_path.is_dir() {
            return coded(
                "invalid_input",
                format!("path is not a directory: {}", search_path.display()),
            );
        }

        if !no_ignore
            && crate::workspace::filter::ignored_discovery_message(
                &execution.workspace_root,
                &search_path,
            )
            .is_some()
        {
            return finish_glob(
                &[],
                file_identity::PathBase::Workspace,
                pattern,
                path_arg,
                no_ignore,
                limit,
                input.get("cursor").and_then(Value::as_str),
                true,
            );
        }

        let results = match glob_match_cancellable(
            &execution.workspace_root,
            &search_path,
            pattern,
            no_ignore,
            Some(&execution.cancel),
        ) {
            Ok(r) => r,
            Err(e) => return crate::tools::file_path::litecode_error(e),
        };
        let base = if outside {
            file_identity::PathBase::Absolute
        } else {
            file_identity::PathBase::Workspace
        };
        finish_glob(
            &results.hits,
            base,
            pattern,
            path_arg,
            no_ignore,
            limit,
            input.get("cursor").and_then(Value::as_str),
            false,
        )
    }
}

#[derive(Debug)]
struct GlobListing {
    hits: Vec<String>,
}

impl GlobListing {
    fn all(hits: Vec<String>) -> Self {
        Self { hits }
    }

    #[cfg(test)]
    fn iter(&self) -> impl Iterator<Item = &String> {
        self.hits.iter()
    }
}

fn parse_glob_limit(input: &Value) -> std::result::Result<usize, String> {
    let Some(value) = input.get("limit").filter(|value| !value.is_null()) else {
        return Ok(DEFAULT_GLOB_LIMIT);
    };
    let Some(limit) = value.as_u64() else {
        return Err("parameter 'limit' must be an integer".into());
    };
    if !(1..=MAX_GLOB_LIMIT as u64).contains(&limit) {
        return Err(format!(
            "parameter 'limit' must be between 1 and {MAX_GLOB_LIMIT}"
        ));
    }
    Ok(limit as usize)
}

fn coded(code: &str, message: impl Into<String>) -> ToolCallResult {
    crate::tools::file_path::coded_error(code, message)
}

fn finish_glob(
    hits: &[String],
    base: file_identity::PathBase,
    pattern: &str,
    path_arg: Option<&str>,
    include_ignored: bool,
    limit: usize,
    cursor: Option<&str>,
    path_excluded: bool,
) -> ToolCallResult {
    let version = file_identity::version_hash(hits.iter().map(String::as_str));
    let bound = match file_identity::bind_cursor(
        cursor,
        &fingerprint(&[
            ("pattern", pattern),
            ("path", path_arg.unwrap_or("")),
            (
                "include_ignored",
                if include_ignored { "true" } else { "false" },
            ),
        ]),
        &version,
    ) {
        Ok(bound) => bound,
        Err(stale) => return stale,
    };
    let offset = bound
        .as_ref()
        .map(|cursor| cursor.offset as usize)
        .unwrap_or(0);
    if offset > hits.len() {
        return file_identity::stale_cursor_result("offset");
    }
    let end = (offset + limit).min(hits.len());
    let page_hits = hits[offset..end].to_vec();
    let has_more = end < hits.len();
    let next = has_more.then(|| {
        encode_cursor(&QueryCursor {
            fp: fingerprint(&[
                ("pattern", pattern),
                ("path", path_arg.unwrap_or("")),
                (
                    "include_ignored",
                    if include_ignored { "true" } else { "false" },
                ),
            ]),
            ver: version,
            offset: end as u64,
            resume_path: None,
            resume_line: None,
        })
    });
    let page = Page {
        has_more,
        next,
        total: Some(hits.len() as u64),
    };
    let reason = if path_excluded {
        Some("path_excluded")
    } else if hits.is_empty() {
        Some("no_match")
    } else {
        None
    };
    let mut data = serde_json::json!({
        "base": base.as_str(),
        "paths": page_hits,
        "page": page,
    });
    if let Some(reason) = reason {
        data["reason"] = serde_json::json!(reason);
    }
    crate::tool::outcome::ok_data(data)
}

fn pattern_reaches_parent(pattern: &str) -> bool {
    let normalized = normalize_pattern(pattern);
    normalized.contains("../") || normalized.starts_with("..")
}

/// Match `pattern` against virtual session paths as given.
/// A repeated directory prefix is not stripped.
fn glob_virtual_sessions(
    execution: &crate::tool::trait_::ToolExecutionContext,
    pattern: &str,
) -> Result<GlobListing> {
    let matcher = compile_include_pattern(pattern)?;
    let reader = execution.session_reader()?;
    let listed = crate::session::transcript_file::list_virtual_paths(
        reader.list_session_ids_blocking().unwrap_or_default(),
    );
    let mut hits: Vec<String> = listed
        .into_iter()
        .filter(|path| matcher.matches(path))
        .collect();
    crate::workspace::sort_glob_hits(&mut hits);
    Ok(GlobListing::all(hits))
}

#[cfg(test)]
fn glob_match(
    workspace: &std::path::Path,
    base: &std::path::Path,
    pattern: &str,
    no_ignore: bool,
) -> Result<GlobListing> {
    glob_match_cancellable(workspace, base, pattern, no_ignore, None)
}

fn glob_match_cancellable(
    workspace: &std::path::Path,
    base: &std::path::Path,
    pattern: &str,
    no_ignore: bool,
    cancel: Option<&tokio_util::sync::CancellationToken>,
) -> Result<GlobListing> {
    let glob_matcher = compile_include_pattern(pattern)?;
    let preset = discovery_preset(no_ignore);
    let rel_ctx = RelPathCtx::new(base).unwrap_or_else(|_| RelPathCtx::new_lossy(base));

    let mut hits: Vec<String> = Vec::new();

    let mut walk_opts = WalkOptions::with_file_include(vec![glob_matcher.clone()]);
    // Glob matches paths, not contents — do not apply the Search binary gate.
    walk_opts.skip_binary = Some(false);
    walk_opts.allow_knowledge_root = true;
    let mut builder = walk_builder_with(base, preset, walk_opts);
    let knowledge = workspace.join(".litecode").join("knowledge");
    if knowledge.is_dir() && cheap_rel_under(&knowledge, base).is_some() {
        builder
            .git_ignore(false)
            .git_global(false)
            .git_exclude(false);
    }
    let walker = builder.build();
    let outside = !crate::config::path::is_under(base, workspace);

    for entry in walker.flatten() {
        if cancel.is_some_and(tokio_util::sync::CancellationToken::is_cancelled) {
            return Err(crate::types::LitecodeError::ToolExecution(
                "glob cancelled".into(),
            ));
        }
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        // Prefer cheap strip (walk already filtered); LAP only on fallback.
        let Some(rel_str) =
            cheap_rel_under(rel_ctx.root_lap(), entry.path()).or_else(|| rel_ctx.rel(entry.path()))
        else {
            continue;
        };
        hits.push(display_glob_path(workspace, base, outside, &rel_str));
    }

    if knowledge.is_dir()
        && cheap_rel_under(base, &knowledge).is_some()
        && cheap_rel_under(&knowledge, base).is_none()
        && let Some(extra) = private_knowledge_walker(workspace, preset, Some(false))
    {
        for entry in extra.build().flatten() {
            if cancel.is_some_and(tokio_util::sync::CancellationToken::is_cancelled) {
                return Err(crate::types::LitecodeError::ToolExecution(
                    "glob cancelled".into(),
                ));
            }
            if !entry.file_type().is_some_and(|kind| kind.is_file()) {
                continue;
            }
            let Some(rel) = cheap_rel_under(base, entry.path()) else {
                continue;
            };
            if !path_matches_include(&rel, std::slice::from_ref(&glob_matcher)) {
                continue;
            }
            let displayed = display_glob_path(workspace, base, outside, &rel);
            if hits.iter().any(|hit| hit == &displayed) {
                continue;
            }
            hits.push(displayed);
        }
    }

    crate::workspace::sort_glob_hits(&mut hits);
    Ok(GlobListing::all(hits))
}

fn display_glob_path(
    workspace: &std::path::Path,
    search_base: &std::path::Path,
    outside: bool,
    rel_to_base: &str,
) -> String {
    let rel_to_base = rel_to_base.replace('\\', "/");
    if outside {
        let mut abs = search_base.to_path_buf();
        for part in rel_to_base.split('/').filter(|part| !part.is_empty()) {
            abs.push(part);
        }
        return file_identity::absolute_path_string(&abs);
    }
    let prefix = cheap_rel_under(workspace, search_base)
        .unwrap_or_default()
        .replace('\\', "/");
    if prefix.is_empty() {
        rel_to_base
    } else if rel_to_base.is_empty() {
        prefix
    } else {
        format!("{prefix}/{rel_to_base}")
    }
}

fn discovery_preset(no_ignore: bool) -> FilterPreset {
    if no_ignore {
        FilterPreset::NoIgnore
    } else {
        FilterPreset::Search
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool::Tool;
    use crate::workspace::filter::compile_include_pattern;
    use tempfile::TempDir;

    #[test]
    fn matcher_supports_braces() {
        let m = compile_include_pattern("file.{rs,toml,json}").unwrap();
        assert!(m.matches("file.rs"));
        assert!(m.matches("file.toml"));
        assert!(!m.matches("file.txt"));
    }

    #[test]
    fn file_glob_skips_search_exclude_keeps_hidden() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::create_dir(root.join(".git")).unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("node_modules/pkg")).unwrap();
        std::fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(root.join("node_modules/pkg/index.js"), "module.exports=1\n").unwrap();
        std::fs::write(root.join(".env"), "AGENT_GLOB=1\n").unwrap();

        let found = glob_match(root, root, "**/*.{rs,js,env}", false)
            .unwrap()
            .hits;
        assert!(
            found
                .iter()
                .any(|p| p == "src/main.rs" || p.ends_with("main.rs")),
            "expected src/main.rs in {found:?}"
        );
        assert!(
            !found.iter().any(|p| p.contains("node_modules")),
            "Search must apply search.exclude; got {found:?}"
        );
        assert!(
            found.iter().any(|p| p == ".env"),
            "Search must not hide dotfiles; got {found:?}"
        );
    }

    #[test]
    fn no_ignore_includes_search_exclude_and_gitignored() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::create_dir(root.join(".git")).unwrap();
        std::fs::write(root.join(".gitignore"), "ignored.txt\n").unwrap();
        std::fs::create_dir_all(root.join("node_modules/pkg")).unwrap();
        std::fs::write(root.join("node_modules/pkg/index.js"), "x\n").unwrap();
        std::fs::write(root.join("ignored.txt"), "x\n").unwrap();
        std::fs::write(root.join("keep.txt"), "x\n").unwrap();

        let filtered = glob_match(root, root, "**/*", false).unwrap().hits;
        assert!(filtered.iter().any(|p| p == "keep.txt"));
        assert!(!filtered.iter().any(|p| p == "ignored.txt"));
        assert!(!filtered.iter().any(|p| p.contains("node_modules")));

        let raw = glob_match(root, root, "**/*", true).unwrap().hits;
        assert!(raw.iter().any(|p| p == "ignored.txt"), "got {raw:?}");
        assert!(
            raw.iter().any(|p| p == "node_modules/pkg/index.js"),
            "got {raw:?}"
        );
    }

    /// PNG signature + IHDR length field (NUL bytes) so `looks_binary` is true.
    fn write_png(path: &std::path::Path) {
        std::fs::write(
            path,
            [
                0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
                0x44, 0x52,
            ],
        )
        .unwrap();
    }

    #[test]
    fn glob_lists_binary_png() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("assets")).unwrap();
        write_png(&root.join("logo.png"));
        write_png(&root.join("assets/icon.png"));
        std::fs::write(root.join("keep.txt"), "x\n").unwrap();

        let from_root = glob_in(root, serde_json::json!({ "pattern": "*.png" }));
        assert!(
            from_root.contains("logo.png"),
            "glob must list binary files by path; got {from_root}"
        );
        assert!(
            from_root.contains("assets/icon.png"),
            "glob must list nested png; got {from_root}"
        );

        let scoped = glob_in(
            root,
            serde_json::json!({ "pattern": "*.png", "path": "assets" }),
        );
        assert!(
            scoped.contains("icon.png"),
            "path=assets must still list png; got {scoped}"
        );
    }

    #[test]
    fn path_into_excluded_dir_is_refused() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::create_dir(root.join(".git")).unwrap();
        std::fs::create_dir_all(root.join("node_modules/pkg")).unwrap();
        std::fs::write(root.join("node_modules/pkg/index.js"), "x\n").unwrap();

        let body = glob_data_in(
            root,
            serde_json::json!({ "pattern": "**/*", "path": "node_modules" }),
        );
        assert_eq!(body["reason"], "path_excluded");
        assert!(body["paths"].as_array().unwrap().is_empty(), "{body}");

        let nested = glob_data_in(
            root,
            serde_json::json!({ "pattern": "**/*", "path": "node_modules/pkg" }),
        );
        assert_eq!(nested["reason"], "path_excluded");
        assert!(nested["paths"].as_array().unwrap().is_empty(), "{nested}");

        let raw = glob_in(
            root,
            serde_json::json!({
                "pattern": "**/*",
                "path": "node_modules",
                "include_ignored": true,
            }),
        );
        assert!(
            raw.contains("pkg/index.js") || raw.contains("index.js"),
            "no_ignore must still list; got {raw}"
        );
    }

    #[test]
    fn glob_reaches_the_knowledge_root_and_refuses_the_rest() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::create_dir(root.join(".git")).unwrap();
        std::fs::write(root.join(".gitignore"), ".litecode/\n").unwrap();
        std::fs::create_dir_all(root.join("knowledge")).unwrap();
        std::fs::write(root.join("knowledge/node.md"), "public\n").unwrap();
        std::fs::create_dir_all(root.join(".litecode/knowledge")).unwrap();
        std::fs::write(root.join(".litecode/knowledge/private.md"), "private\n").unwrap();
        std::fs::create_dir_all(root.join(".litecode/index")).unwrap();
        std::fs::write(root.join(".litecode/index/x.rs"), "fn x() {}\n").unwrap();
        std::fs::write(root.join("keep.rs"), "fn k() {}\n").unwrap();

        let filtered = glob_match(root, root, "**/*", false).unwrap();
        assert!(
            filtered.iter().any(|path| path == "keep.rs"),
            "{filtered:?}"
        );
        assert!(
            filtered.iter().any(|path| path == "knowledge/node.md"),
            "{filtered:?}"
        );
        assert!(
            filtered
                .iter()
                .any(|path| path == ".litecode/knowledge/private.md"),
            "{filtered:?}"
        );
        assert!(
            !filtered.iter().any(|path| path.contains("index")),
            "{filtered:?}"
        );

        let index = glob_data_in(
            root,
            serde_json::json!({ "pattern": "**/*", "path": ".litecode/index" }),
        );
        assert_eq!(index["reason"], "path_excluded");
        assert!(index["paths"].as_array().unwrap().is_empty(), "{index}");

        let inside = glob_in(
            root,
            serde_json::json!({ "pattern": "**/*", "path": ".litecode" }),
        );
        assert!(
            inside.contains("knowledge/private.md"),
            "path=.litecode lists the knowledge root; got {inside}"
        );
        assert!(
            !inside.contains("index"),
            "path=.litecode does not list the rest; got {inside}"
        );
    }

    #[test]
    fn parent_dir_pattern_is_invalid_input() {
        assert!(pattern_reaches_parent("../*"));
        assert!(pattern_reaches_parent("foo/../bar"));
        assert!(!pattern_reaches_parent("**/*.rs"));
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("a.rs"), "fn a() {}\n").unwrap();
        let result = glob_result_in(dir.path(), serde_json::json!({ "pattern": "../*" }));
        assert_eq!(result.error_code.as_deref(), Some("invalid_input"));
        assert!(
            result
                .content
                .contains("pattern '../*' reaches outside the search directory"),
            "{}",
            result.content
        );
        assert!(result.data.is_none());
    }

    #[test]
    fn repeated_path_prefix_is_not_stripped() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src/agent")).unwrap();
        std::fs::write(root.join("src/agent/core.rs"), "fn x() {}\n").unwrap();

        let found = glob_match(root, &root.join("src"), "src/**/*.rs", false)
            .unwrap()
            .hits;
        assert!(
            found.is_empty(),
            "pattern is matched under path as given, got {found:?}"
        );
        let found = glob_match(root, &root.join("src"), "**/*.rs", false)
            .unwrap()
            .hits;
        assert!(
            found.iter().any(|path| path.ends_with("src/agent/core.rs")),
            "got {found:?}"
        );
    }

    #[test]
    fn hits_sort_by_depth_then_parent_then_name() {
        let mut hits = vec![
            "src/tools/read.rs".into(),
            "src/b.rs".into(),
            "z.md".into(),
            "src/tools/glob.rs".into(),
            "a.md".into(),
            "src/a.rs".into(),
            "tests/a.rs".into(),
        ];
        crate::workspace::sort_glob_hits(&mut hits);
        assert_eq!(
            hits,
            [
                "a.md",
                "z.md",
                "src/a.rs",
                "src/b.rs",
                "tests/a.rs",
                "src/tools/glob.rs",
                "src/tools/read.rs",
            ]
        );
    }

    #[test]
    fn glob_match_view_is_depth_grouped_not_mtime() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src/nested")).unwrap();
        std::fs::write(root.join("src/nested/deep.rs"), "fn d() {}\n").unwrap();
        std::fs::write(root.join("root.rs"), "fn r() {}\n").unwrap();
        std::fs::write(root.join("src/mid.rs"), "fn m() {}\n").unwrap();

        let found = glob_match(root, root, "**/*.rs", false).unwrap();
        assert_eq!(found.hits, ["root.rs", "src/mid.rs", "src/nested/deep.rs"]);
        assert_eq!(found.hits.len(), 3);
    }

    #[test]
    fn truncates_with_count_note() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        for i in 0..(DEFAULT_GLOB_LIMIT + 7) {
            std::fs::write(root.join(format!("f{i:04}.txt")), "x\n").unwrap();
        }
        let listing = glob_match(root, root, "*.txt", true).unwrap();
        assert_eq!(listing.hits.len(), DEFAULT_GLOB_LIMIT + 7);
        let data = glob_data_in(root, serde_json::json!({"pattern": "*.txt"}));
        assert_eq!(data["paths"].as_array().unwrap().len(), DEFAULT_GLOB_LIMIT);
        assert_eq!(data["page"]["has_more"], true);
        assert_eq!(data["page"]["total"], listing.hits.len() as u64);
    }

    #[test]
    fn basename_glob_recurses() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src/nested")).unwrap();
        std::fs::write(root.join("src/nested/deep.rs"), "fn d() {}\n").unwrap();
        std::fs::write(root.join("root.rs"), "fn r() {}\n").unwrap();
        std::fs::write(root.join("src/mid.rs"), "fn m() {}\n").unwrap();
        std::fs::write(root.join("src/nested/README.md"), "# nested\n").unwrap();

        let rs = glob_match(root, root, "*.rs", false).unwrap();
        assert!(rs.iter().any(|p| p == "src/nested/deep.rs"), "{rs:?}");
        assert!(rs.iter().any(|p| p == "src/mid.rs"), "{rs:?}");
        assert!(rs.iter().any(|p| p == "root.rs"), "{rs:?}");

        let md = glob_match(root, root, "README.md", false).unwrap();
        assert!(
            md.iter().any(|p| p == "src/nested/README.md"),
            "exact basename must recurse; got {md:?}"
        );
    }

    #[test]
    fn empty_glob_sets_no_match_reason() {
        let dir = TempDir::new().unwrap();
        let result = glob_result_in(dir.path(), serde_json::json!({ "pattern": "**/*.nope" }));
        let data = result.data.expect("data");
        assert!(data["paths"].as_array().unwrap().is_empty(), "{data}");
        assert_eq!(data["reason"], "no_match");
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(
            !result.content.contains("excludes.json")
                && !result.content.contains("include_ignored"),
            "{}",
            result.content
        );
    }

    fn seed_session(root: &std::path::Path, text: &str) -> String {
        let db = root.join(".litecode").join("sessions.db");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        let lease = crate::session::WorkspaceWriteLease::acquire(db.parent().unwrap()).unwrap();
        let data = crate::session::SessionData::open(&lease, &db).unwrap();
        let id = data
            .create_session(root.to_str().unwrap(), "default", None)
            .unwrap();
        data.insert_items(&id, &[crate::types::user_text(text)])
            .unwrap();
        id
    }

    fn glob_result_in(root: &std::path::Path, input: Value) -> ToolCallResult {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");
        rt.block_on(GlobTool.execute(
            input,
            crate::tool::trait_::ToolExecutionContext {
                path_mode: crate::workspace::ToolPathMode::Safe,
                workspace_root: root.to_path_buf(),
                call_id: String::new(),
                cancel: tokio_util::sync::CancellationToken::new(),
                output_limit: GlobTool.max_result_size(),
                session_id: String::new(),
                session: Some(crate::session::SessionDataReader::open(
                    &root.join(".litecode").join("sessions.db"),
                )),
            },
        ))
    }

    fn glob_in(root: &std::path::Path, input: Value) -> String {
        let data = glob_data_in(root, input);
        data["paths"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|path| path.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn glob_data_in(root: &std::path::Path, input: Value) -> Value {
        glob_result_in(root, input)
            .data
            .expect("glob must return structured data")
    }

    #[test]
    fn structured_result_lists_paths_and_totals() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "a").unwrap();
        std::fs::write(root.join("b.txt"), "b").unwrap();
        std::fs::write(root.join("c.md"), "c").unwrap();

        let data = glob_data_in(root, serde_json::json!({"pattern": "**/*.txt"}));
        assert_eq!(data["base"], "workspace");
        assert_eq!(data["page"]["total"], 2);
        assert_eq!(data["page"]["has_more"], false);
        let mut paths: Vec<String> = data["paths"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap().to_string())
            .collect();
        paths.sort();
        assert_eq!(paths, vec!["a.txt".to_string(), "b.txt".to_string()]);
    }

    #[test]
    fn structured_result_is_empty_on_no_hits() {
        let dir = TempDir::new().unwrap();
        let data = glob_data_in(dir.path(), serde_json::json!({"pattern": "**/*.nope"}));
        assert_eq!(data["paths"], serde_json::json!([]));
        assert_eq!(data["page"]["total"], 0);
        assert_eq!(data["page"]["has_more"], false);
    }

    #[test]
    fn include_ignored_is_the_public_boolean() {
        assert_eq!(GlobTool.timeout(), Some(Duration::from_secs(30)));
        assert!(GlobTool.is_cancellable());
        let schema = GlobTool.schema();
        let props = schema["properties"].as_object().unwrap();
        assert!(props.contains_key("include_ignored"));
        assert!(!props.contains_key("-u"));

        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::create_dir(root.join(".git")).unwrap();
        std::fs::write(root.join(".gitignore"), "ignored.txt\n").unwrap();
        std::fs::write(root.join("ignored.txt"), "x\n").unwrap();
        let data = glob_data_in(
            root,
            serde_json::json!({"pattern": "**/*", "include_ignored": true}),
        );
        assert!(
            data["paths"]
                .as_array()
                .unwrap()
                .iter()
                .any(|path| path == "ignored.txt")
        );
    }

    #[test]
    fn structured_result_reports_truncation() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        for i in 0..(DEFAULT_GLOB_LIMIT + 3) {
            std::fs::write(root.join(format!("f{i:04}.txt")), "x").unwrap();
        }
        let data = glob_data_in(root, serde_json::json!({"pattern": "*.txt"}));
        assert_eq!(data["paths"].as_array().unwrap().len(), DEFAULT_GLOB_LIMIT);
        assert_eq!(data["page"]["total"], DEFAULT_GLOB_LIMIT + 3);
        assert_eq!(data["page"]["has_more"], true);
        assert!(data["page"]["next"].is_string());
    }

    #[test]
    fn virtual_sessions_list_only_when_path_is_explicit() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let a = seed_session(root, "alpha");
        let b = seed_session(root, "beta");
        let path_a = crate::session::transcript_file::virtual_path_for(&a);
        let path_b = crate::session::transcript_file::virtual_path_for(&b);

        let listed = glob_in(
            root,
            serde_json::json!({
                "pattern": "*.md",
                "path": ".litecode/sessions",
            }),
        );
        assert!(listed.contains(&path_a), "{listed}");
        assert!(listed.contains(&path_b), "{listed}");

        let unscoped = glob_in(root, serde_json::json!({ "pattern": "**/*.md" }));
        assert!(
            !unscoped.contains(".litecode/sessions/"),
            "coding glob must not enter sessions, got: {unscoped}"
        );

        let pattern_only = glob_in(
            root,
            serde_json::json!({ "pattern": ".litecode/sessions/*.md" }),
        );
        assert!(
            !pattern_only.contains(&path_a),
            "pattern without path must not list sessions, got: {pattern_only}"
        );
    }

    fn session_md_hits(listed: &str) -> Vec<&str> {
        listed
            .lines()
            .filter(|l| l.starts_with(".litecode/sessions/") && l.ends_with(".md"))
            .collect()
    }

    #[test]
    fn explicit_session_dir_glob_lists_every_session() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let ids: Vec<String> = (0..3)
            .map(|i| seed_session(root, &format!("glob body {i}")))
            .collect();
        let expected: Vec<String> = ids
            .iter()
            .map(|id| crate::session::transcript_file::virtual_path_for(id))
            .collect();

        for input in [
            serde_json::json!({ "pattern": "**/*", "path": ".litecode/sessions" }),
            serde_json::json!({ "pattern": "*.md", "path": ".litecode/sessions" }),
            serde_json::json!({ "pattern": "*.md", "path": ".litecode/sessions/" }),
            serde_json::json!({
                "pattern": ".litecode/sessions/*.md",
                "path": ".litecode/sessions",
            }),
        ] {
            let listed = glob_in(root, input.clone());
            let hits = session_md_hits(&listed);
            assert_eq!(
                hits.len(),
                3,
                "explicit dir glob must list all sessions for {input}:\n{listed}"
            );
            for path in &expected {
                assert!(hits.contains(&path.as_str()), "missing {path} in {listed}");
            }
        }

        let parent = glob_in(
            root,
            serde_json::json!({ "pattern": "**/*", "path": ".litecode" }),
        );
        for path in &expected {
            assert!(
                !parent.contains(path),
                "parent .litecode must not list virtual transcripts, got: {parent}"
            );
        }

        let unscoped_ignore = glob_in(
            root,
            serde_json::json!({ "pattern": "**/*.md", "include_ignored": true }),
        );
        assert!(
            !unscoped_ignore.contains(".litecode/sessions/"),
            "no_ignore workspace glob must still skip sessions, got: {unscoped_ignore}"
        );
    }

    #[test]
    fn data_schema_accepts_structured_results() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "a").unwrap();
        let data = glob_data_in(dir.path(), serde_json::json!({"pattern": "**/*.txt"}));
        let schema = GlobTool.data_schema().expect("glob data schema");
        crate::tool::schema_validate(&schema, &data)
            .unwrap_or_else(|error| panic!("glob data does not match its schema: {error}: {data}"));
    }

    #[test]
    fn cancelled_glob_stops_before_walking() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("a.rs"), "x\n").unwrap();
        let cancel = tokio_util::sync::CancellationToken::new();
        cancel.cancel();
        let error = glob_match_cancellable(dir.path(), dir.path(), "**/*.rs", false, Some(&cancel))
            .unwrap_err();
        assert!(error.to_string().contains("glob cancelled"), "{error}");
    }
}

use std::time::Duration;

use serde_json::Value;

use crate::tool::Tool;
use crate::tool::file_identity;
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
            return finish_glob(&results.hits, limit, outside);
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

        if crate::workspace::filter::ignored_discovery_message(
            &execution.workspace_root,
            &search_path,
        )
        .is_some()
        {
            let mut result = finish_glob(&[], limit, outside);
            if let Some(data) = result.data.as_mut() {
                data["message"] = serde_json::json!(
                    "Path is hidden by default search filters (.gitignore / workspace excludes)."
                );
            }
            return result;
        }

        let results = match glob_match_cancellable(
            &execution.workspace_root,
            &search_path,
            pattern,
            false,
            Some(&execution.cancel),
        ) {
            Ok(r) => r,
            Err(e) => return crate::tools::file_path::litecode_error(e),
        };
        finish_glob(&results.hits, limit, outside)
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

fn finish_glob(hits: &[String], limit: usize, outside: bool) -> ToolCallResult {
    let truncated = hits.len() > limit;
    let page_hits = if truncated {
        hits[..limit].to_vec()
    } else {
        hits.to_vec()
    };
    let mut message = String::new();
    if truncated {
        message = format!(
            "Showing first {} of {} paths. Raise limit to see more.",
            page_hits.len(),
            hits.len()
        );
    } else if hits.is_empty() {
        message = "No matching paths.".into();
    }
    if outside {
        let note = "Paths outside the workspace are absolute.";
        if message.is_empty() {
            message = note.into();
        } else {
            message = format!("{message} {note}");
        }
    }
    crate::tool::outcome::ok_data(serde_json::json!({
        "paths": page_hits,
        "message": message,
    }))
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

    use tempfile::TempDir;

    #[test]
    fn default_filters_hide_gitignored() {
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
        assert!(raw.iter().any(|p| p == "ignored.txt"), "{raw:?}");
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
                .contains("reaches outside the search directory"),
            "{}",
            result.content
        );
        assert!(result.data.is_none());
    }

    #[test]
    fn truncates_with_message() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        for i in 0..(DEFAULT_GLOB_LIMIT + 7) {
            std::fs::write(root.join(format!("f{i:04}.txt")), "x\n").unwrap();
        }
        let listing = glob_match(root, root, "*.txt", true).unwrap();
        assert_eq!(listing.hits.len(), DEFAULT_GLOB_LIMIT + 7);
        let data = glob_data_in(root, serde_json::json!({"pattern": "*.txt"}));
        assert_eq!(data["paths"].as_array().unwrap().len(), DEFAULT_GLOB_LIMIT);
        assert!(
            data["message"].as_str().unwrap().contains("Raise limit"),
            "{}",
            data["message"]
        );
        assert!(data.get("page").is_none());
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
        assert!(md.iter().any(|p| p == "src/nested/README.md"), "{md:?}");
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

    fn glob_data_in(root: &std::path::Path, input: Value) -> Value {
        glob_result_in(root, input)
            .data
            .expect("glob must return structured data")
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

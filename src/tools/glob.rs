use serde_json::Value;

use crate::context_pipeline::Context;
use crate::tool::Tool;
use crate::tool::trait_::ToolExecutionContext;
use crate::types::{Result, ToolCallResult};
use crate::workspace::filter::{
    FilterPreset, RelPathCtx, WalkOptions, cheap_rel_under, compile_include_pattern,
    normalize_pattern, walk_builder_with,
};

const MAX_RESULTS: usize = 1000;

pub struct GlobTool;

impl Tool for GlobTool {
    fn name(&self) -> &str {
        "glob"
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Glob relative to path (or workspace root if path omitted). Example: '**/*.rs'. Do not repeat path — if path is 'src', use '**/*.rs' not 'src/**/*.rs'."
                },
                "path": {
                    "type": "string",
                    "description": "Optional directory to search under (workspace-relative preferred; absolute paths outside the workspace only under All permission). Pattern is matched relative to this directory."
                },
                "no_ignore": {
                    "type": "boolean",
                    "description": "When true, walk without .gitignore / files.exclude / search.exclude (default: false). Use to discover ignored paths such as build outputs."
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
        self.call_for_execution(
            input,
            ToolExecutionContext {
                path_mode: crate::workspace::ToolPathMode::All,
                workspace_root: crate::config::workspace::workspace_root_lap(),
                call_id: String::new(),
                cancel: tokio_util::sync::CancellationToken::new(),
                output_limit: self.max_result_size(),
                session_id: String::new(),
            },
        )
    }

    fn description(&self, _ctx: &Context) -> String {
        "Find files by path glob; optional `path` scopes to another directory (workspace-relative preferred).".into()
    }
}

impl GlobTool {
    fn call_for_execution(&self, input: Value, execution: ToolExecutionContext) -> ToolCallResult {
        let pattern = match crate::tool::require_nonempty_string(&input, "pattern") {
            Ok(s) => s,
            Err(e) => return ToolCallResult::error(e),
        };

        let pattern_warning = parent_dir_pattern_warning(pattern);
        let no_ignore = input["no_ignore"].as_bool().unwrap_or(false);
        let path_arg = input["path"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty());

        let (effective_pattern, prefix_note) = strip_redundant_path_prefix(path_arg, pattern);

        let search_path = match path_arg {
            Some(path) => {
                match crate::workspace::resolve_agent(
                    &execution.workspace_root,
                    path,
                    execution.path_mode,
                ) {
                    Ok(path) => path,
                    Err(error) => return ToolCallResult::error(error.to_string()),
                }
            }
            None => execution.workspace_root.clone(),
        };

        if !search_path.exists() {
            return ToolCallResult::error(format!(
                "path does not exist: {}",
                search_path.display()
            ));
        }
        if !search_path.is_dir() {
            return ToolCallResult::error(format!(
                "path is not a directory: {}",
                search_path.display()
            ));
        }

        let results = match glob_match(&search_path, &effective_pattern, no_ignore) {
            Ok(r) => r,
            Err(e) => return ToolCallResult::error(e.to_string()),
        };

        let body = if results.is_empty() {
            let mut msg = format!("No files found for pattern '{effective_pattern}'");
            if let Some(p) = path_arg {
                msg.push_str(&format!(" under path '{p}'"));
                msg.push_str(". Pattern is relative to path — with path set, prefer '**/*.rs' over repeating the directory in pattern.");
            }
            if let Some(w) = pattern_warning {
                msg.push_str(". ");
                msg.push_str(&w);
                msg.push_str(" Glob only matches paths under the search directory; patterns like '../*' cannot reach parent folders.");
            }
            if let Some(note) = prefix_note {
                msg.push_str(". ");
                msg.push_str(&note);
            }
            msg
        } else {
            let mut msg = results.join("\n");
            if let Some(note) = prefix_note {
                msg.push('\n');
                msg.push_str(&note);
            }
            msg
        };
        ToolCallResult::ok(body)
    }
}

fn parent_dir_pattern_warning(pattern: &str) -> Option<String> {
    let normalized = normalize_pattern(pattern);
    if normalized.contains("../") || normalized.starts_with("..") {
        Some(format!(
            "pattern '{pattern}' references parent directories; workspace-relative paths never include '..', so matches are usually empty."
        ))
    } else {
        None
    }
}

/// When `path` is `src` and `pattern` is `src/**/*.rs`, match against `**/*.rs`.
///
/// `glob_match` compares the pattern to paths *relative to* `path`, so repeating
/// the directory prefix never hits. Agents commonly pass both; strip quietly.
fn strip_redundant_path_prefix<'a>(
    path: Option<&str>,
    pattern: &'a str,
) -> (std::borrow::Cow<'a, str>, Option<String>) {
    let Some(path) = path else {
        return (std::borrow::Cow::Borrowed(pattern), None);
    };
    let path_n = normalize_pattern(path).trim_matches('/').trim().to_string();
    if path_n.is_empty() || path_n == "." {
        return (std::borrow::Cow::Borrowed(pattern), None);
    }
    let pattern_n = normalize_pattern(pattern);
    let prefix = format!("{path_n}/");
    if let Some(rest) = pattern_n.strip_prefix(&prefix) {
        let rest = if rest.is_empty() { "**/*" } else { rest };
        let note = format!(
            "stripped redundant '{path_n}/' from pattern (matched as '{rest}' under path '{path_n}')"
        );
        return (std::borrow::Cow::Owned(rest.to_string()), Some(note));
    }
    if pattern_n == path_n {
        let note = format!("pattern equaled path '{path_n}'; using '**/*' under that directory");
        return (std::borrow::Cow::Borrowed("**/*"), Some(note));
    }
    (std::borrow::Cow::Borrowed(pattern), None)
}

fn glob_match(base: &std::path::Path, pattern: &str, no_ignore: bool) -> Result<Vec<String>> {
    let glob_matcher = compile_include_pattern(pattern)?;
    let preset = discovery_preset(no_ignore);
    let rel_ctx = RelPathCtx::new(base).unwrap_or_else(|_| RelPathCtx::new_lossy(base));

    let mut hits: Vec<String> = Vec::new();

    let mut builder = walk_builder_with(
        base,
        preset,
        WalkOptions::with_file_include(vec![glob_matcher]),
    );
    if shallow_only(pattern) {
        builder.max_depth(Some(1));
    }
    let walker = builder.build();

    for entry in walker.flatten() {
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        // Prefer cheap strip (walk already filtered); LAP only on fallback.
        let Some(rel_str) =
            cheap_rel_under(rel_ctx.root_lap(), entry.path()).or_else(|| rel_ctx.rel(entry.path()))
        else {
            continue;
        };
        hits.push(rel_str);
    }

    crate::workspace::sort_glob_hits(&mut hits);
    hits.truncate(MAX_RESULTS);
    Ok(hits)
}

fn discovery_preset(no_ignore: bool) -> FilterPreset {
    if no_ignore {
        FilterPreset::Unfiltered
    } else {
        FilterPreset::FileGlob
    }
}

/// Patterns without `**` or path separators only match the start directory.
fn shallow_only(pattern: &str) -> bool {
    let pattern = normalize_pattern(pattern);
    !pattern.contains("**") && !pattern.contains('/')
}

#[cfg(test)]
mod tests {
    use super::*;
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

        let found = glob_match(root, "**/*.{rs,js,env}", false).unwrap();
        assert!(
            found
                .iter()
                .any(|p| p == "src/main.rs" || p.ends_with("main.rs")),
            "expected src/main.rs in {found:?}"
        );
        assert!(
            !found.iter().any(|p| p.contains("node_modules")),
            "FileGlob must apply search.exclude; got {found:?}"
        );
        assert!(
            found.iter().any(|p| p == ".env"),
            "FileGlob must not hide dotfiles; got {found:?}"
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

        let filtered = glob_match(root, "**/*", false).unwrap();
        assert!(filtered.iter().any(|p| p == "keep.txt"));
        assert!(!filtered.iter().any(|p| p == "ignored.txt"));
        assert!(!filtered.iter().any(|p| p.contains("node_modules")));

        let raw = glob_match(root, "**/*", true).unwrap();
        assert!(raw.iter().any(|p| p == "ignored.txt"), "got {raw:?}");
        assert!(
            raw.iter().any(|p| p == "node_modules/pkg/index.js"),
            "got {raw:?}"
        );
    }

    #[test]
    fn path_into_excluded_dir_still_lists() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::create_dir(root.join(".git")).unwrap();
        std::fs::create_dir_all(root.join("node_modules/pkg")).unwrap();
        std::fs::write(root.join("node_modules/pkg/index.js"), "x\n").unwrap();

        let found = glob_match(&root.join("node_modules"), "**/*", false).unwrap();
        assert!(
            found
                .iter()
                .any(|p| p == "pkg/index.js" || p.ends_with("index.js")),
            "walking path=node_modules should list contents; got {found:?}"
        );
    }

    #[test]
    fn parent_dir_pattern_warning_detects_dot_dot() {
        assert!(parent_dir_pattern_warning("../*").is_some());
        assert!(parent_dir_pattern_warning("foo/../bar").is_some());
        assert!(parent_dir_pattern_warning("**/*.rs").is_none());
        let text = parent_dir_pattern_warning("../*").unwrap();
        assert!(
            text.ends_with('.'),
            "warning must end with a period so it does not glue to the next sentence: {text}"
        );
    }

    #[test]
    fn strips_redundant_path_prefix_from_pattern() {
        let (p, note) = strip_redundant_path_prefix(Some("src"), "src/**/*.rs");
        assert_eq!(p, "**/*.rs");
        assert!(note.unwrap().contains("stripped redundant"));

        let (p, note) = strip_redundant_path_prefix(Some("src/"), "src/**/*.rs");
        assert_eq!(p, "**/*.rs");
        assert!(note.is_some());

        let (p, note) = strip_redundant_path_prefix(Some("src"), "**/*.rs");
        assert_eq!(p, "**/*.rs");
        assert!(note.is_none());

        let (p, _) = strip_redundant_path_prefix(None, "src/**/*.rs");
        assert_eq!(p, "src/**/*.rs");

        let (p, note) = strip_redundant_path_prefix(Some("src"), "src");
        assert_eq!(p, "**/*");
        assert!(note.is_some());
    }

    #[test]
    fn path_plus_prefixed_pattern_finds_files() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src/agent")).unwrap();
        std::fs::write(root.join("src/agent/core.rs"), "fn x() {}\n").unwrap();

        // After strip: path=src + pattern=src/**/*.rs → **/*.rs under src
        let (effective, _) = strip_redundant_path_prefix(Some("src"), "src/**/*.rs");
        let found = glob_match(&root.join("src"), &effective, false).unwrap();
        assert!(
            found
                .iter()
                .any(|p| p == "agent/core.rs" || p.ends_with("core.rs")),
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

        let found = glob_match(root, "**/*.rs", false).unwrap();
        assert_eq!(found, ["root.rs", "src/mid.rs", "src/nested/deep.rs"]);
    }
}

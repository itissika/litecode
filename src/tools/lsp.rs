use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::time::{Duration, Instant};

use futures_util::Future;
use serde_json::Value;

use crate::config::path::strip_verbatim;
use crate::context_pipeline::Context;
use crate::engines::{EngineState, WorkspaceEngines};
use crate::lsp::SharedLspHub;
use crate::tool::Tool;
use crate::tool::trait_::ToolExecutionContext;
use crate::types::ToolCallResult;

/// Agent-facing LSP operations (first-class navigation + diagnostics + hover).
/// Hub/editor still support a wider action set; this tool deliberately does not.
const OPERATIONS: &[&str] = &[
    "goToDefinition",
    "findReferences",
    "hover",
    "diagnostics",
    // Legacy aliases (still accepted)
    "definition",
    "references",
];

/// How long a Warming engine may block before we fail (not "empty result").
const LSP_WARM_WAIT: Duration = Duration::from_secs(30);
const LSP_WARM_POLL: Duration = Duration::from_millis(100);

/// ± context around a landing location when syntax-ancestor expansion fails (grep-aligned).
const LANDING_CONTEXT_LINES: u32 = 2;

/// Per-block line cap when fan-out yields multiple in-line text hits.
const MULTI_HIT_BLOCK_MAX_LINES: usize = 50;

/// Characters after the needle included in a multi-hit `##` heading.
const HIT_HEADING_RIGHT_CHARS: usize = 5;

pub struct LspTool {
    hub: SharedLspHub,
    engines: WorkspaceEngines,
    workspace_root: PathBuf,
}

impl LspTool {
    pub fn new(engines: WorkspaceEngines, workspace_root: PathBuf) -> Self {
        Self {
            hub: engines.lsp_hub(),
            engines,
            workspace_root: strip_verbatim(&workspace_root),
        }
    }

    fn availability_failure(&self, workspace_root: &Path) -> Option<String> {
        match self.engines.state("lsp") {
            Some(EngineState::Warm) => None,
            Some(EngineState::Warming) => Some(
                "LSP is still loading for the current workspace after waiting; \
                 skipped — use read/grep, or wait for Settings → Engines → LSP to show Warm, then retry"
                    .into(),
            ),
            Some(EngineState::Failed) => Some(format!(
                "LSP is unavailable in the current workspace (root={}): {}. \
                 Check Settings → Engines → LSP, then retry",
                workspace_root.display(),
                self.engines
                    .last_error("lsp")
                    .unwrap_or_else(|| "language engine failed to start".into())
            )),
            _ => Some(format!(
                "LSP is not enabled for the current workspace (root={}). \
                 Configure and start an LSP server in Settings → Engines → LSP",
                workspace_root.display()
            )),
        }
    }

    fn is_ready(&self) -> bool {
        self.engines.state("lsp") == Some(EngineState::Warm)
    }

    /// Per-server gate: the umbrella `state("lsp")` only reflects the default
    /// engine. A *different* language's server can be Failed while the umbrella
    /// is Warm, so the tool must check the per-server lifecycle for the target
    /// file's extension before calling (2.11/LspTool gate).
    fn per_server_failure(&self, file_path: &Path) -> Option<String> {
        let ext = file_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default();
        let command = crate::lsp::server_map::server_command_for_ext(ext)?;
        let program = crate::lsp::server_map::program_from_command(&command);
        let detail = failed_server_detail(&self.hub.instance_statuses(), &program)?;
        Some(format!(
            "LSP server for '{program}' failed: {detail}. Check Settings → Engines → LSP, then retry"
        ))
    }

    /// Block while Warming (human loading spinner). Fail only on timeout / Failed / disabled.
    async fn ensure_ready(
        &self,
        workspace_root: &Path,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> Result<(), ToolCallResult> {
        if self.is_ready() {
            return Ok(());
        }

        match self.engines.state("lsp") {
            Some(EngineState::Warming) => {}
            _ => {
                let msg = self
                    .availability_failure(workspace_root)
                    .expect("not ready");
                return Err(ToolCallResult::error(msg));
            }
        }

        let started = Instant::now();
        while started.elapsed() < LSP_WARM_WAIT {
            if cancel.is_cancelled() {
                return Err(ToolCallResult::error("lsp cancelled while waiting for LSP"));
            }
            if self.is_ready() {
                return Ok(());
            }
            if self.engines.state("lsp") == Some(EngineState::Failed) {
                let msg = self.availability_failure(workspace_root).expect("failed");
                return Err(ToolCallResult::error(msg));
            }
            if !matches!(self.engines.state("lsp"), Some(EngineState::Warming)) {
                break;
            }
            tokio::time::sleep(LSP_WARM_POLL).await;
        }

        if self.is_ready() {
            return Ok(());
        }
        let msg = self
            .availability_failure(workspace_root)
            .expect("still not ready");
        Err(ToolCallResult::error(msg))
    }
}

/// Find the failure detail for `program` among per-server statuses (2.11 gate).
fn failed_server_detail(
    statuses: &[crate::lsp::LspInstanceStatus],
    program: &str,
) -> Option<String> {
    statuses
        .iter()
        .find(|s| s.command.contains(program) && s.state == crate::lsp::LspLifecycle::Failed)
        .map(|s| {
            s.last_error
                .clone()
                .unwrap_or_else(|| "language server failed".into())
        })
}

fn scope_guidance(root: &Path, got: &str) -> String {
    format!(
        "path not in LSP workspace scope (root={}). Got '{}'. \
         Pass a workspace-relative path (e.g. src/main.rs) or an absolute path under that root.",
        root.display(),
        got
    )
}

/// Resolve `file_path` against the LSP workspace root. Out-of-scope paths fail here
/// so the call never reaches the language server.
fn resolve_in_workspace(workspace_root: &Path, path_str: &str) -> Result<PathBuf, String> {
    let trimmed = path_str.trim();
    if trimmed.is_empty() {
        return Err(crate::tool::must_be_nonempty_string("file_path"));
    }

    let root = crate::config::path::canon_abs_lossy(workspace_root);
    let resolved = crate::workspace::resolve_lsp_workspace(&root, trimmed)
        .map_err(|_| scope_guidance(&root, trimmed))?;
    if !resolved.exists() {
        return Err(format!(
            "file not found: {trimmed} (resolved under workspace {})",
            root.display()
        ));
    }
    if !resolved.is_file() {
        return Err(format!(
            "lsp requires a real source file, got '{}' (resolved {}). \
             Pass a workspace-relative file path (e.g. src/main.rs).",
            path_str.trim(),
            resolved.display()
        ));
    }
    Ok(resolved)
}

/// One substring hit of `text` on a source line (for LSP column + multi-hit view).
#[derive(Debug, Clone)]
struct TextHit {
    /// 1-based UTF-16 column at the start of the match (editor-style).
    column: u64,
    /// Byte offset into the source line.
    byte_start: usize,
}

#[derive(Debug, Clone)]
struct TextResolve {
    source_line: String,
    needle: String,
    hits: Vec<TextHit>,
}

/// Resolve Agent `line` + `text` into all 1-based UTF-16 columns on that line.
///
/// Mental model: file + line (from read) + text snippet. Zero hits is a hard
/// positioning failure; multiple hits fan out to separate LSP queries.
fn resolve_text_on_line(file_path: &Path, line: u64, text: &str) -> Result<TextResolve, String> {
    if line == 0 {
        return Err(crate::tool::must_be("line", ">= 1 (1-indexed)"));
    }
    let needle = text.trim();
    if needle.is_empty() {
        return Err(crate::tool::must_be_nonempty_string("text"));
    }

    let content = std::fs::read(file_path).map_err(|e| {
        format!(
            "cannot read {} to resolve text position: {e}",
            file_path.display()
        )
    })?;
    let decoded = crate::workspace::text_codec::decode_utf8_bytes(&content).map_err(|e| {
        crate::workspace::text_codec::decode_error_for_path(e, &file_path.display().to_string())
    })?;
    let lines: Vec<&str> = decoded.text.lines().collect();
    let Some(source_line) = lines.get((line - 1) as usize) else {
        return Err(format!(
            "line {line} is outside the file ({} lines in {})",
            lines.len(),
            file_path.display()
        ));
    };

    let mut hits: Vec<TextHit> = Vec::new();
    let mut search_from = 0usize;
    while let Some(rel) = source_line[search_from..].find(needle) {
        let byte_start = search_from + rel;
        let utf16 = source_line[..byte_start].encode_utf16().count() as u64;
        hits.push(TextHit {
            column: utf16 + 1,
            byte_start,
        });
        search_from = byte_start + needle.len().max(1);
        if search_from >= source_line.len() {
            break;
        }
    }

    if hits.is_empty() {
        return Err(format!(
            "text `{needle}` not found on line {line} of {}:\n  {source_line}\n\
             Copy an exact substring from the read tool output for that line.",
            file_path.display()
        ));
    }

    Ok(TextResolve {
        source_line: (*source_line).to_string(),
        needle: needle.to_string(),
        hits,
    })
}

/// Multi-hit block title: `##` + needle + up to 5 Unicode scalars to the right.
fn hit_heading(source_line: &str, byte_start: usize, needle: &str) -> String {
    let end = (byte_start + needle.len()).min(source_line.len());
    // Keep needle as provided when byte slice is valid; otherwise fall back.
    let hit = source_line
        .get(byte_start..end)
        .filter(|s| *s == needle)
        .unwrap_or(needle);
    let right: String = source_line
        .get(end..)
        .unwrap_or("")
        .chars()
        .take(HIT_HEADING_RIGHT_CHARS)
        .collect();
    format!("##{hit}{right}")
}

fn fence_block(snippet: &str) -> String {
    let mut longest = 2usize;
    let mut run = 0usize;
    for ch in snippet.chars() {
        if ch == '`' {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    let ticks = "`".repeat(longest + 1);
    format!("{ticks}\n{snippet}\n{ticks}\n")
}

fn numbered_line_window(content: &str, hit_line: u32, before: u32, after: u32) -> String {
    let lines: Vec<&str> = content.lines().collect();
    if lines.is_empty() || hit_line == 0 {
        return String::new();
    }
    let idx = (hit_line as usize).saturating_sub(1);
    if idx >= lines.len() {
        return String::new();
    }
    let start = idx.saturating_sub(before as usize);
    let end = (idx + after as usize + 1).min(lines.len());
    let mut out = String::new();
    for (i, line) in lines[start..end].iter().enumerate() {
        let line_no = start + i + 1;
        out.push_str(&format!("{line_no:>4}|{line}\n"));
    }
    out
}

fn snippet_for_landing(path: &Path, line: u32) -> String {
    let Ok(src) = std::fs::read_to_string(path) else {
        return String::new();
    };
    let path_str = path.to_string_lossy();
    if let Some(ancestor) =
        crate::engines::code_search::syntax_ancestor_snippet(&path_str, &src, line, line)
    {
        let text =
            crate::engines::code_search::lines_slice(&src, ancestor.start_line, ancestor.end_line);
        let mut out = fence_block(&text);
        if ancestor.remaining_lines > 0 {
            out.push_str(&format!(
                "{} lines remaining in ancestor node. Read the file to see all.\n",
                ancestor.remaining_lines
            ));
        }
        return out;
    }
    numbered_line_window(&src, line, LANDING_CONTEXT_LINES, LANDING_CONTEXT_LINES)
}

fn parse_location_line(s: &str) -> Option<(PathBuf, u32, u32)> {
    let (path_line, col_s) = s.rsplit_once(':')?;
    let (path_s, line_s) = path_line.rsplit_once(':')?;
    let line: u32 = line_s.parse().ok()?;
    let col: u32 = col_s.parse().ok()?;
    if line == 0 {
        return None;
    }
    Some((PathBuf::from(path_s), line, col))
}

/// Attach grep-style context under each `path:line:col` for definition / references.
fn enrich_nav_result(raw: &str) -> String {
    if raw.starts_with("No locations found") {
        return raw.to_string();
    }
    let mut locations: Vec<(String, PathBuf, u32, u32)> = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some((path, ln, col)) = parse_location_line(trimmed) {
            locations.push((trimmed.to_string(), path, ln, col));
        } else {
            // Non-location payload — leave untouched.
            return raw.to_string();
        }
    }
    if locations.is_empty() {
        return raw.to_string();
    }

    let mut out = String::new();
    for (label, path, ln, _col) in &locations {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(label);
        out.push('\n');
        let snippet = snippet_for_landing(path, *ln);
        if !snippet.is_empty() {
            out.push_str(&snippet);
            if !out.ends_with('\n') {
                out.push('\n');
            }
        }
    }
    out
}

fn enrich_action_result(action: &str, raw: &str) -> String {
    match action {
        "goToDefinition" | "definition" | "findReferences" | "references" => enrich_nav_result(raw),
        _ => raw.to_string(),
    }
}

fn clip_block_lines(body: &str, max_lines: usize) -> String {
    let lines: Vec<&str> = body.lines().collect();
    if lines.len() <= max_lines {
        return body.to_string();
    }
    let mut out = lines[..max_lines].join("\n");
    out.push_str(&format!(
        "\n… truncated ({} more lines). Narrow `text` and retry, or read the file.\n",
        lines.len() - max_lines
    ));
    out
}

fn format_multi_hit_view(
    line: u64,
    source_line: &str,
    needle: &str,
    hits: &[TextHit],
    blocks: &[String],
    // When true (goToDefinition), identical landing bodies share one expansion.
    merge_identical_bodies: bool,
) -> String {
    let n = hits.len();
    debug_assert_eq!(hits.len(), blocks.len());

    let groups: Vec<(usize, Vec<usize>)> = if merge_identical_bodies {
        let mut groups: Vec<(usize, Vec<usize>)> = Vec::new();
        for (i, block) in blocks.iter().enumerate() {
            let key = block.trim();
            if let Some((_, idxs)) = groups.iter_mut().find(|(bi, _)| blocks[*bi].trim() == key) {
                idxs.push(i);
            } else {
                groups.push((i, vec![i]));
            }
        }
        groups
    } else {
        (0..hits.len()).map(|i| (i, vec![i])).collect()
    };

    let unique = groups.len();
    let mut out = if merge_identical_bodies && unique < n {
        format!(
            "text `{needle}` matched {n} times on line {line} → {unique} unique definition{}\n",
            if unique == 1 { "" } else { "s" }
        )
    } else {
        format!("text `{needle}` matched {n} times on line {line}\n")
    };

    for (body_idx, hit_idxs) in groups {
        out.push('\n');
        for &i in &hit_idxs {
            out.push_str(&hit_heading(source_line, hits[i].byte_start, needle));
            out.push('\n');
        }
        let clipped = clip_block_lines(&blocks[body_idx], MULTI_HIT_BLOCK_MAX_LINES);
        out.push_str(&clipped);
        if !out.ends_with('\n') {
            out.push('\n');
        }
    }
    out
}

fn definition_merges_identical_landings(action: &str) -> bool {
    matches!(action, "goToDefinition" | "definition")
}

fn map_lsp_query_error(err: impl std::fmt::Display) -> ToolCallResult {
    let msg = err.to_string();
    if msg.contains("index not ready") || msg.contains("inconclusive") {
        ToolCallResult::error(format!(
            "{msg}. Wait for Settings → Engines → LSP to finish indexing, then retry"
        ))
    } else {
        ToolCallResult::error(msg)
    }
}

fn needs_position(action: &str) -> bool {
    matches!(
        action,
        "definition" | "goToDefinition" | "references" | "findReferences" | "hover"
    )
}

impl Tool for LspTool {
    fn name(&self) -> &str {
        "lsp"
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": [
                        "goToDefinition",
                        "findReferences",
                        "hover",
                        "diagnostics"
                    ],
                    "description": "goToDefinition: jump to symbol definition. findReferences: list references. hover: type/docs at position. diagnostics: file diagnostics. Prefer: goToDefinition. Avoid: inventing action names (e.g. workspaceSymbol)."
                },
                "file_path": {
                    "type": "string",
                    "description": "Source file under the workspace. Prefer workspace-relative (e.g. src/main.rs); absolute path under the root also works. Prefer: src/lib.rs. Avoid: directories, paths outside the workspace, or non-files."
                },
                "line": {
                    "type": "integer",
                    "description": "1-based line number from a prior read of the file. Prefer: the exact line shown by read. Avoid: 0-based indexes, guessed lines, or lines from an outdated buffer."
                },
                "text": {
                    "type": "string",
                    "description": "Exact substring on the target line identifying the symbol; tool resolves column(s). Prefer: copy the symbol token from read output (e.g. AgentDeps). Avoid: invented spellings, fuzzy abbreviations, or pasting the entire line."
                }
            },
            "required": ["action", "file_path"]
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    fn timeout(&self) -> Option<u64> {
        // Warm wait (≤30s) + LSP request headroom.
        Some(60)
    }

    fn max_result_size(&self) -> usize {
        20_000
    }

    fn validate_input(&self, input: &Value) -> std::result::Result<(), String> {
        let action = crate::tool::require_nonempty_string(input, "action")?;
        crate::tool::require_nonempty_string(input, "file_path")?;
        if !OPERATIONS.contains(&action) {
            return Err(crate::tool::must_be_one_of("action", OPERATIONS, action));
        }
        if needs_position(action) {
            match input.get("line") {
                None => return Err(crate::tool::missing_parameter("line")),
                Some(v) => {
                    let line = v
                        .as_u64()
                        .ok_or_else(|| crate::tool::expected_type("line", "integer", v))?;
                    if line < 1 {
                        return Err(crate::tool::must_be("line", ">= 1 (1-indexed)"));
                    }
                }
            }
            crate::tool::require_nonempty_string_trimmed(input, "text")?;
        }
        Ok(())
    }

    /// LSP operations go through LspHub's sole async exit (`run_on_hub`).
    fn execute(
        &self,
        input: Value,
        execution: ToolExecutionContext,
    ) -> Pin<Box<dyn Future<Output = ToolCallResult> + Send + '_>> {
        Box::pin(self.call_for_execution(input, execution))
    }

    fn call_async(
        &self,
        input: Value,
    ) -> Pin<Box<dyn Future<Output = ToolCallResult> + Send + '_>> {
        // Unit-test / sync helper path: use construction-time workspace root.
        // Production Agent turns enter through execute() with ToolExecutionContext.
        self.execute(
            input,
            ToolExecutionContext {
                path_mode: crate::workspace::ToolPathMode::Safe,
                workspace_root: self.workspace_root.clone(),
                call_id: String::new(),
                cancel: tokio_util::sync::CancellationToken::new(),
                output_limit: self.max_result_size(),
                session_id: String::new(),
            },
        )
    }

    fn description(&self, _ctx: &Context) -> String {
        "Language-server navigation and diagnostics: goToDefinition, findReferences, \
         hover, diagnostics. Read the file first to obtain line numbers, then call. \
         diagnostics needs file_path only; other actions need file_path + line + text. \
         Waits briefly if the language server is still loading."
            .into()
    }
}

impl LspTool {
    async fn call_for_execution(
        &self,
        input: Value,
        execution: ToolExecutionContext,
    ) -> ToolCallResult {
        // LSP paths are always SAFE workspace-scoped; path_mode from the turn is ignored.
        let workspace_root = crate::config::path::canon_abs_lossy(&execution.workspace_root);
        if let Err(e) = self.validate_input(&input) {
            return ToolCallResult::error(e);
        }
        let action = input["action"].as_str().expect("validated").to_string();
        let file_path_str = input["file_path"].as_str().expect("validated").to_string();

        let file_path = match resolve_in_workspace(&workspace_root, &file_path_str) {
            Ok(p) => p,
            Err(msg) => return ToolCallResult::error(msg),
        };

        // Per-server gate: fail fast when the target file's language server is
        // Failed, regardless of the umbrella engine state.
        if let Some(msg) = self.per_server_failure(&file_path) {
            return ToolCallResult::error(msg);
        }

        if let Err(err) = self.ensure_ready(&workspace_root, &execution.cancel).await {
            return err;
        }

        let position = if needs_position(&action) {
            let line = input["line"].as_u64().expect("validated line");
            let text = input["text"].as_str().expect("validated text");
            match resolve_text_on_line(&file_path, line, text) {
                Ok(resolved) => Some((line, resolved)),
                Err(msg) => return ToolCallResult::error(msg),
            }
        } else {
            None
        };

        let hub = self.hub.clone();
        // diagnostics (and any non-position action): single query, no fan-out.
        let Some((line, resolved)) = position else {
            return match hub
                .tool_action_with_query(&action, &file_path, None, None, None)
                .await
            {
                Ok(text) => ToolCallResult::ok(text),
                Err(e) => map_lsp_query_error(e),
            };
        };

        let multi = resolved.hits.len() >= 2;
        let mut blocks: Vec<String> = Vec::with_capacity(resolved.hits.len());
        for hit in &resolved.hits {
            let raw = match hub
                .tool_action_with_query(&action, &file_path, Some(line), Some(hit.column), None)
                .await
            {
                Ok(text) => text,
                Err(e) if !multi => return map_lsp_query_error(e),
                Err(e) => e.to_string(),
            };
            blocks.push(enrich_action_result(&action, &raw));
        }

        if !multi {
            return ToolCallResult::ok(blocks.pop().unwrap_or_default());
        }

        let body = format_multi_hit_view(
            line,
            &resolved.source_line,
            &resolved.needle,
            &resolved.hits,
            &blocks,
            definition_merges_identical_landings(&action),
        );
        ToolCallResult::ok(body).with_warning("narrow text and retry if you need a single hit")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_path_resolves_under_root() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src")).unwrap();
        let file = root.join("src/lib.rs");
        std::fs::write(&file, "fn x() {}\n").unwrap();

        let resolved = resolve_in_workspace(root, "src/lib.rs").unwrap();
        assert_eq!(resolved, crate::config::path::canon_abs(&file).unwrap());
    }

    #[test]
    fn absolute_under_root_ok() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let file = root.join("main.rs");
        std::fs::write(&file, "fn main() {}\n").unwrap();

        let resolved = resolve_in_workspace(root, file.to_str().unwrap()).unwrap();
        assert_eq!(resolved, crate::config::path::canon_abs(&file).unwrap());
    }

    #[test]
    fn absolute_outside_root_rejected_with_guidance() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let outside = tempfile::tempdir().unwrap();
        let file = outside.path().join("other.rs");
        std::fs::write(&file, "fn y() {}\n").unwrap();

        let err = resolve_in_workspace(root, file.to_str().unwrap()).unwrap_err();
        assert!(
            err.contains("path not in LSP workspace scope"),
            "got: {err}"
        );
        assert!(err.contains("workspace-relative"), "got: {err}");
    }

    #[test]
    fn parent_dir_escape_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let err = resolve_in_workspace(dir.path(), "../secret.rs").unwrap_err();
        assert!(
            err.contains("path not in LSP workspace scope"),
            "got: {err}"
        );
    }

    #[test]
    fn missing_relative_reports_not_found_not_lsp() {
        let dir = tempfile::tempdir().unwrap();
        let err = resolve_in_workspace(dir.path(), "missing.rs").unwrap_err();
        assert!(err.contains("file not found"), "got: {err}");
        assert!(!err.contains("LSP hub"), "got: {err}");
    }

    #[test]
    fn directory_rejected_for_all_actions() {
        let dir = tempfile::tempdir().unwrap();
        let err = resolve_in_workspace(dir.path(), ".").unwrap_err();
        assert!(err.contains("requires a real source file"), "got: {err}");
    }

    #[test]
    fn text_on_line_resolves_utf16_column() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("lib.rs");
        std::fs::write(&file, "fn main() {}\n").unwrap();

        // `main` starts after "fn " → UTF-16 cols 1='f',2='n',3=' ',4='m'
        let resolved = resolve_text_on_line(&file, 1, "main").unwrap();
        assert_eq!(resolved.hits.len(), 1);
        assert_eq!(resolved.hits[0].column, 4);
        assert!(resolve_text_on_line(&file, 1, "missing").is_err());
        assert!(resolve_text_on_line(&file, 0, "main").is_err());
        assert!(resolve_text_on_line(&file, 2, "main").is_err());
    }

    #[test]
    fn text_on_line_returns_all_hits() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("lib.rs");
        std::fs::write(&file, "foo foo\n").unwrap();

        let resolved = resolve_text_on_line(&file, 1, "foo").unwrap();
        assert_eq!(resolved.hits.len(), 2);
        assert_eq!(resolved.hits[0].column, 1);
        assert_eq!(resolved.hits[1].column, 5);
    }

    #[test]
    fn hit_heading_includes_right_five_chars() {
        let line = "let foo = foo_bar;";
        // first `foo` at byte 4 → ##foo = f
        assert_eq!(hit_heading(line, 4, "foo"), "##foo = fo");
        // second `foo` inside foo_bar at byte 10 → ##foo_bar; (right 5)
        assert_eq!(hit_heading(line, 10, "foo"), "##foo_bar;");
    }

    #[test]
    fn format_multi_hit_merges_independent_blocks() {
        let line = "foo foo";
        let hits = vec![
            TextHit {
                column: 1,
                byte_start: 0,
            },
            TextHit {
                column: 5,
                byte_start: 4,
            },
        ];
        let blocks = vec![
            "No locations found (language server ready; no definition/reference at this position)."
                .into(),
            "src/a.rs:1:1\n".into(),
        ];
        let view = format_multi_hit_view(1, line, "foo", &hits, &blocks, false);
        assert!(view.contains("matched 2 times"), "got: {view}");
        assert!(view.contains("##foo foo"), "got: {view}");
        assert!(view.contains("##foo"), "got: {view}");
        assert!(view.contains("No locations found"), "got: {view}");
        assert!(view.contains("src/a.rs:1:1"), "got: {view}");
        // Without merge, each block keeps its own body even if we later add identical cases.
        assert_eq!(
            view.matches("##foo").count(),
            2,
            "two separate expansions: {view}"
        );
    }

    #[test]
    fn format_multi_hit_merges_identical_definition_landings() {
        let line = "target(); target();";
        let hits = vec![
            TextHit {
                column: 1,
                byte_start: 0,
            },
            TextHit {
                column: 11,
                byte_start: 10,
            },
        ];
        let landing = "src/lib.rs:1:8\n```\npub fn target() {}\n```\n".to_string();
        let blocks = vec![landing.clone(), landing];
        let view = format_multi_hit_view(4, line, "target", &hits, &blocks, true);

        assert!(
            view.contains("matched 2 times on line 4 → 1 unique definition"),
            "got: {view}"
        );
        assert!(view.contains("##target(); t"), "got: {view}");
        assert!(view.contains("##target();"), "got: {view}");
        // One expansion only.
        assert_eq!(
            view.matches("src/lib.rs:1:8").count(),
            1,
            "identical landings must collapse to one body: {view}"
        );
        assert_eq!(view.matches("pub fn target").count(), 1, "got: {view}");
    }

    #[test]
    fn format_multi_hit_keeps_distinct_definition_landings() {
        let line = "a a";
        let hits = vec![
            TextHit {
                column: 1,
                byte_start: 0,
            },
            TextHit {
                column: 3,
                byte_start: 2,
            },
        ];
        let blocks = vec!["src/a.rs:1:1\n".into(), "src/b.rs:2:1\n".into()];
        let view = format_multi_hit_view(1, line, "a", &hits, &blocks, true);
        assert!(
            view.contains("matched 2 times on line 1\n"),
            "no → unique when landings differ: {view}"
        );
        assert!(!view.contains("unique definition"), "got: {view}");
        assert!(view.contains("src/a.rs:1:1"), "got: {view}");
        assert!(view.contains("src/b.rs:2:1"), "got: {view}");
    }

    #[test]
    fn clip_block_lines_truncates_multi_hit_blocks() {
        let body = (1..=60)
            .map(|i| format!("line{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let clipped = clip_block_lines(&body, MULTI_HIT_BLOCK_MAX_LINES);
        assert!(clipped.contains("truncated"), "got: {clipped}");
        assert_eq!(clipped.lines().count(), MULTI_HIT_BLOCK_MAX_LINES + 1);
        let short = "a\nb\n";
        assert_eq!(clip_block_lines(short, MULTI_HIT_BLOCK_MAX_LINES), short);
    }

    #[test]
    fn enrich_nav_falls_back_to_plus_minus_two() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("notes.txt");
        std::fs::write(&file, "one\ntwo\nthree\nfour\nfive\n").unwrap();
        let label = format!("{}:3:1", file.display());
        let enriched = enrich_nav_result(&label);
        assert!(enriched.contains(&label), "got: {enriched}");
        // ±2 around line 3 → lines 1..5
        assert!(enriched.contains("1|one"), "got: {enriched}");
        assert!(enriched.contains("3|three"), "got: {enriched}");
        assert!(enriched.contains("5|five"), "got: {enriched}");
    }

    #[test]
    fn enrich_nav_prefers_ancestor_fence_for_rust() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("lib.rs");
        std::fs::write(&file, "fn outer() {\n    let x = 1;\n    let y = 2;\n}\n").unwrap();
        let label = format!("{}:2:9", file.display());
        let enriched = enrich_nav_result(&label);
        assert!(enriched.contains(&label), "got: {enriched}");
        // Ancestor should fence the fn body (or at least include surrounding lines).
        assert!(
            enriched.contains("fn outer") || enriched.contains("let x"),
            "got: {enriched}"
        );
        assert!(
            enriched.contains("```"),
            "rust ancestor should use a fenced block: {enriched}"
        );
    }

    #[test]
    fn enrich_action_result_views_differ_by_action() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("notes.txt");
        std::fs::write(&file, "a\nb\nc\nd\ne\n").unwrap();
        let loc = format!("{}:3:1", file.display());

        let def = enrich_action_result("goToDefinition", &loc);
        assert!(def.contains(&loc), "got: {def}");
        assert!(
            def.contains("3|c"),
            "definition should attach ±2 context: {def}"
        );
        assert!(
            !def.contains("##"),
            "single-location enrich must not invent multi-hit headings: {def}"
        );

        let refs = enrich_action_result("findReferences", &loc);
        assert!(refs.contains("3|c"), "references share nav enrich: {refs}");

        let hover = enrich_action_result("hover", "fn item\nDocs here");
        assert_eq!(
            hover, "fn item\nDocs here",
            "hover must stay raw (no location enrich)"
        );

        let empty = enrich_action_result(
            "goToDefinition",
            "No locations found (language server ready; no definition/reference at this position).",
        );
        assert!(
            empty.starts_with("No locations found"),
            "empty nav stays plain English: {empty}"
        );
    }

    #[test]
    fn multi_hit_view_shape_is_readable_and_clips_blocks() {
        let line = "alpha alpha";
        let hits = vec![
            TextHit {
                column: 1,
                byte_start: 0,
            },
            TextHit {
                column: 7,
                byte_start: 6,
            },
        ];
        let long = (1..=55)
            .map(|i| format!("L{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let blocks = vec![
            "No locations found (language server ready; no definition/reference at this position.)"
                .into(),
            long,
        ];
        let view = format_multi_hit_view(3, line, "alpha", &hits, &blocks, false);

        let summary = view.lines().next().unwrap();
        assert_eq!(summary, "text `alpha` matched 2 times on line 3");

        // Headings: needle + right 5 (" alph" / end-of-line empty)
        assert!(
            view.contains("##alpha alph"),
            "first heading should include right context: {view}"
        );
        assert!(
            view.contains("\n##alpha\n") || view.contains("\n##alpha\r"),
            "second heading at EOL is ##alpha: {view}"
        );

        // Independent blocks: empty/error block preserved; long block clipped.
        assert!(view.contains("No locations found"), "got: {view}");
        assert!(view.contains("truncated"), "second block must clip: {view}");
        assert!(
            !view.contains("L55") || view.contains("truncated"),
            "clipped body should not keep the full tail unmarked: {view}"
        );

        // Order: summary, then heading1 before heading2 content markers.
        let h1 = view.find("##alpha alph").expect("h1");
        let h2 = view
            .rfind("\n##alpha\n")
            .or_else(|| view.rfind("\n##alpha"))
            .expect("h2");
        assert!(h1 < h2, "headings out of order in: {view}");
    }

    #[test]
    fn single_hit_path_has_no_multi_hit_skeleton() {
        // Document the contract: multi skeleton is only via format_multi_hit_view.
        let line = "only_once";
        let hits = [TextHit {
            column: 1,
            byte_start: 0,
        }];
        // Callers must not wrap N=1 in format_multi_hit_view; if they did, skeleton appears.
        // This asserts the helper's heading shape so integration can rely on absence for Ok.
        assert_eq!(hit_heading(line, 0, "only_once"), "##only_once");
        let view = format_multi_hit_view(1, line, "only_once", &hits, &["body".into()], false);
        assert!(view.contains("matched 1 times"), "got: {view}");
    }

    #[test]
    fn text_on_line_handles_multibyte_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("lib.rs");
        // `😀` is two UTF-16 units; `b` starts at 1-based column 4 (a=1, emoji=2..3, b=4).
        std::fs::write(&file, "a😀b\n").unwrap();
        let resolved = resolve_text_on_line(&file, 1, "b").unwrap();
        assert_eq!(resolved.hits[0].column, 4);
    }

    #[test]
    fn validate_rejects_removed_actions_and_missing_text() {
        let engines = WorkspaceEngines::new();
        let tool = LspTool::new(engines, PathBuf::from("."));

        assert!(
            tool.validate_input(&serde_json::json!({
                "action": "workspaceSymbol",
                "file_path": "src/main.rs",
            }))
            .is_err()
        );

        assert!(
            tool.validate_input(&serde_json::json!({
                "action": "goToDefinition",
                "file_path": "src/main.rs",
                "line": 1,
            }))
            .is_err()
        );

        assert!(
            tool.validate_input(&serde_json::json!({
                "action": "goToDefinition",
                "file_path": "src/main.rs",
                "line": 1,
                "text": "main",
            }))
            .is_ok()
        );

        assert!(
            tool.validate_input(&serde_json::json!({
                "action": "diagnostics",
                "file_path": "src/main.rs",
            }))
            .is_ok()
        );
    }

    #[test]
    fn resolve_strips_windows_verbatim_after_canonicalize_shape() {
        // Simulate the Windows canonicalize form without requiring a real \\?\ filesystem.
        let verbatim = PathBuf::from(r"\\?\E:\litecode\src\lib.rs");
        let root = PathBuf::from(r"E:\litecode");
        assert!(
            !verbatim.starts_with(&root),
            "precondition: verbatim must not match stripped root"
        );
        let normalized = strip_verbatim(&verbatim);
        assert!(
            normalized.starts_with(&root),
            "normalized={normalized:?} root={root:?}"
        );
    }

    #[test]
    fn per_server_gate_matches_only_failed_program() {
        use crate::lsp::{LspInstanceStatus, LspLifecycle};
        fn status(command: &str, state: LspLifecycle) -> LspInstanceStatus {
            LspInstanceStatus {
                command: command.into(),
                project_root: "/proj".into(),
                state,
                index_settled: false,
                last_error: None,
                restart_count: 0,
            }
        }
        // Failed server for the target program is reported.
        let statuses = vec![
            status("rust-analyzer", LspLifecycle::Running),
            status("pyright", LspLifecycle::Failed),
        ];
        assert!(failed_server_detail(&statuses, "pyright").is_some());
        // Running / other programs are not reported.
        assert!(failed_server_detail(&statuses, "rust-analyzer").is_none());
        assert!(failed_server_detail(&statuses, "gopls").is_none());
        // Failed detail carries the last_error when present.
        let mut failed = status("pyright", LspLifecycle::Failed);
        failed.last_error = Some("boom".into());
        assert_eq!(
            failed_server_detail(&[failed], "pyright").as_deref(),
            Some("boom")
        );
    }
}

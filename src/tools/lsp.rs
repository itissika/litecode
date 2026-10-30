//! Code-surface LSP tools (WP-202): seven independent snake_case tools.
//!
//! Catalog card id remains `"lsp"` (engine Ready + agent bind). Instantiation
//! injects these seven Code tools — Direct never exposes a tool named `"lsp"`.
//! Positioning is `path` + 1-based `line` + 1-based UTF-16 `character`
//! (no `text` substring resolve). `workspace_symbol` takes `{ query }` only.

use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::Future;
use serde_json::Value;

use crate::config::path::strip_verbatim;
use crate::engines::{EngineState, WorkspaceEngines};
use crate::lsp::SharedLspHub;
use crate::lsp::format::{
    MAX_DIAGNOSTIC_ITEMS, MAX_LOCATIONS, MAX_SYMBOLS, extract_diagnostic_items,
    extract_hover_markdown, extract_location_items, extract_symbol_items,
};
use crate::tool::Tool;
use crate::tool::trait_::ToolExecutionContext;
use crate::types::ToolCallResult;

const LSP_WARM_WAIT: Duration = Duration::from_secs(30);
const LSP_WARM_POLL: Duration = Duration::from_millis(100);

const CONTENT_MODIFIED_WARNING: &str = "language server is updating (ContentModified); result may be incomplete — retry in a few seconds, do not treat as 'not found'";

/// Catalog / bind id (settings card). Not a Direct tool name.
pub const LSP_CARD_ID: &str = "lsp";

/// Seven Code tool names injected when the `lsp` card is bound.
pub const LSP_CODE_TOOL_NAMES: &[&str] = &[
    "go_to_definition",
    "find_references",
    "hover",
    "diagnostics",
    "document_symbol",
    "workspace_symbol",
    "go_to_implementation",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LspOp {
    GoToDefinition,
    FindReferences,
    Hover,
    Diagnostics,
    DocumentSymbol,
    WorkspaceSymbol,
    GoToImplementation,
}

impl LspOp {
    fn name(self) -> &'static str {
        match self {
            Self::GoToDefinition => "go_to_definition",
            Self::FindReferences => "find_references",
            Self::Hover => "hover",
            Self::Diagnostics => "diagnostics",
            Self::DocumentSymbol => "document_symbol",
            Self::WorkspaceSymbol => "workspace_symbol",
            Self::GoToImplementation => "go_to_implementation",
        }
    }

    fn hub_action(self) -> &'static str {
        match self {
            Self::GoToDefinition => "goToDefinition",
            Self::FindReferences => "findReferences",
            Self::Hover => "hover",
            Self::Diagnostics => "diagnostics",
            Self::DocumentSymbol => "documentSymbol",
            Self::WorkspaceSymbol => "workspaceSymbol",
            Self::GoToImplementation => "goToImplementation",
        }
    }

    fn needs_position(self) -> bool {
        matches!(
            self,
            Self::GoToDefinition | Self::FindReferences | Self::Hover | Self::GoToImplementation
        )
    }
}

#[derive(Clone)]
struct LspShared {
    hub: SharedLspHub,
    engines: WorkspaceEngines,
    workspace_root: PathBuf,
}

impl LspShared {
    fn new(engines: WorkspaceEngines, workspace_root: PathBuf) -> Self {
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
                 skipped — wait for Settings → Engines → LSP to show Warm, then retry"
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

    async fn ensure_ready(
        &self,
        workspace_root: &Path,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> Result<(), ToolCallResult> {
        let _ = self.engines.ensure_lsp_probed().await;
        if self.is_ready() {
            return Ok(());
        }
        match self.engines.state("lsp") {
            Some(EngineState::Warming) => {}
            _ => {
                let msg = self
                    .availability_failure(workspace_root)
                    .expect("not ready");
                return Err(coded("unavailable", msg));
            }
        }
        let started = Instant::now();
        while started.elapsed() < LSP_WARM_WAIT {
            if cancel.is_cancelled() {
                return Err(coded("cancelled", "lsp cancelled while waiting for LSP"));
            }
            if self.is_ready() {
                return Ok(());
            }
            if self.engines.state("lsp") == Some(EngineState::Failed) {
                let msg = self.availability_failure(workspace_root).expect("failed");
                return Err(coded("unavailable", msg));
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
        Err(coded("unavailable", msg))
    }
}

/// One of the seven Code-surface LSP tools.
pub struct LspOpTool {
    shared: LspShared,
    op: LspOp,
}

impl LspOpTool {
    fn new(shared: LspShared, op: LspOp) -> Self {
        Self { shared, op }
    }
}

/// Build the seven Code tools for a bound `lsp` catalog card.
pub fn all_code_tools(engines: WorkspaceEngines, workspace_root: PathBuf) -> Vec<Arc<dyn Tool>> {
    let shared = LspShared::new(engines, workspace_root);
    [
        LspOp::GoToDefinition,
        LspOp::FindReferences,
        LspOp::Hover,
        LspOp::Diagnostics,
        LspOp::DocumentSymbol,
        LspOp::WorkspaceSymbol,
        LspOp::GoToImplementation,
    ]
    .into_iter()
    .map(|op| Arc::new(LspOpTool::new(shared.clone(), op)) as Arc<dyn Tool>)
    .collect()
}

impl Tool for LspOpTool {
    fn name(&self) -> &str {
        self.op.name()
    }

    fn surface(&self) -> crate::tool::ToolSurface {
        crate::tool::ToolSurface::Code
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    fn timeout(&self) -> Option<Duration> {
        Some(Duration::from_secs(60))
    }

    fn max_result_size(&self) -> usize {
        20_000
    }

    fn validate_input(&self, input: &Value) -> std::result::Result<(), String> {
        match self.op {
            LspOp::WorkspaceSymbol => {
                crate::tool::require_nonempty_string_trimmed(input, "query")?;
                Ok(())
            }
            LspOp::Diagnostics | LspOp::DocumentSymbol => {
                require_lsp_path(input)?;
                Ok(())
            }
            _ => {
                require_lsp_path(input)?;
                let line = require_u64_ge(input, "line", 1)?;
                let character = require_u64_ge(input, "character", 1)?;
                let _ = (line, character);
                Ok(())
            }
        }
    }

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
        self.execute(
            input,
            ToolExecutionContext {
                path_mode: crate::workspace::ToolPathMode::Safe,
                workspace_root: self.shared.workspace_root.clone(),
                call_id: String::new(),
                cancel: tokio_util::sync::CancellationToken::new(),
                output_limit: self.max_result_size(),
                session_id: String::new(),
                session: None,
            },
        )
    }
}

impl LspOpTool {
    async fn call_for_execution(
        &self,
        input: Value,
        execution: ToolExecutionContext,
    ) -> ToolCallResult {
        let workspace_root = crate::config::path::canon_abs_lossy(&execution.workspace_root);
        if let Err(e) = self.validate_input(&input) {
            return coded("invalid_input", e);
        }

        if let Err(err) = self
            .shared
            .ensure_ready(&workspace_root, &execution.cancel)
            .await
        {
            return err;
        }

        // WP-206 ⑧: canonical root first (file URIs we send are canonical), then
        // the configured root; prefix-only, symlinks are not resolved.
        let configured_root = self.shared.workspace_root.clone();
        let roots: [&Path; 2] = [workspace_root.as_path(), configured_root.as_path()];

        if self.op == LspOp::WorkspaceSymbol {
            let query = input["query"].as_str().expect("validated").to_string();
            return match self.shared.hub.workspace_symbol_outcome(&query).await {
                Ok(out) => finish_symbols(out, true, &roots),
                Err(e) => map_lsp_query_error(e),
            };
        }

        let file_path_str = lsp_path(&input).expect("validated").to_string();
        let file_path = match resolve_in_workspace(&workspace_root, &file_path_str) {
            Ok(p) => p,
            Err(msg) => return coded("invalid_input", msg),
        };

        if let Some(msg) = self.shared.per_server_failure(&file_path) {
            return coded("unavailable", msg);
        }

        let (line, character) = if self.op.needs_position() {
            let line = input["line"].as_u64().expect("validated");
            let character = input["character"].as_u64().expect("validated");
            if let Err(msg) = validate_position_on_file(&file_path, line, character) {
                return coded("invalid_input", msg);
            }
            (Some(line), Some(character))
        } else {
            (None, None)
        };

        let display_path = file_path_str;
        match self
            .shared
            .hub
            .tool_action_outcome(self.op.hub_action(), &file_path, line, character, None)
            .await
        {
            Ok(out) => match self.op {
                LspOp::Hover => finish_hover(out),
                LspOp::Diagnostics => finish_diagnostics(out, &display_path),
                LspOp::DocumentSymbol => finish_symbols(out, false, &roots),
                LspOp::WorkspaceSymbol => unreachable!("handled above"),
                _ => finish_locations(out, &roots),
            },
            Err(e) => map_lsp_query_error(e),
        }
    }
}

fn coded(code: &str, message: impl Into<String>) -> ToolCallResult {
    crate::tools::file_path::coded_error(code, message)
}

fn require_u64_ge(input: &Value, name: &str, min: u64) -> Result<u64, String> {
    match input.get(name) {
        None => Err(crate::tool::missing_parameter(name)),
        Some(v) => {
            let n = v
                .as_u64()
                .ok_or_else(|| crate::tool::expected_type(name, "integer", v))?;
            if n < min {
                return Err(crate::tool::must_be(name, &format!(">= {min} (1-indexed)")));
            }
            Ok(n)
        }
    }
}

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

fn lsp_path(input: &Value) -> Option<&str> {
    input.get("path").and_then(Value::as_str)
}

fn require_lsp_path(input: &Value) -> Result<&str, String> {
    match lsp_path(input) {
        Some(path) if !path.is_empty() => Ok(path),
        Some(_) => Err(crate::tool::must_be_nonempty_string("path")),
        None => Err(crate::tool::missing_parameter("path")),
    }
}

fn resolve_in_workspace(workspace_root: &Path, path_str: &str) -> Result<PathBuf, String> {
    let trimmed = path_str.trim();
    if trimmed.is_empty() {
        return Err(crate::tool::must_be_nonempty_string("path"));
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

/// 1-based UTF-16 `character` must lie on the line (caret after last char OK).
fn validate_position_on_file(file_path: &Path, line: u64, character: u64) -> Result<(), String> {
    let content = std::fs::read(file_path).map_err(|e| {
        format!(
            "cannot read {} to validate position: {e}",
            file_path.display()
        )
    })?;
    let decoded = crate::workspace::text_codec::decode_utf8_bytes(&content).map_err(|e| {
        crate::workspace::text_codec::decode_error_for_path(e, &file_path.display().to_string())
    })?;
    let lines: Vec<&str> = decoded.text.lines().collect();
    if line == 0 || line as usize > lines.len() {
        return Err(format!(
            "line {line} out of range for {} ({} lines)",
            file_path.display(),
            lines.len()
        ));
    }
    let source_line = lines[(line as usize) - 1];
    let utf16_len = source_line.encode_utf16().count() as u64;
    // Allow caret at end-of-line (UTF-16 length + 1 in 1-based form).
    if character < 1 || character > utf16_len + 1 {
        return Err(format!(
            "character {character} out of range on {} of {} (line UTF-16 length {utf16_len}; valid 1..={})",
            crate::tool::format_line_label(line as u32, line as u32),
            file_path.display(),
            utf16_len + 1
        ));
    }
    Ok(())
}

fn with_content_modified(result: ToolCallResult, content_modified: bool) -> ToolCallResult {
    if content_modified {
        result.with_warning(CONTENT_MODIFIED_WARNING)
    } else {
        result
    }
}

fn map_lsp_query_error(err: impl std::fmt::Display) -> ToolCallResult {
    let msg = err.to_string();
    let code = if msg.contains("index not ready") || msg.contains("inconclusive") {
        "unavailable"
    } else if msg.contains("not found") || msg.contains("file not found") {
        "not_found"
    } else {
        "unavailable"
    };
    let msg = if msg.contains("index not ready") || msg.contains("inconclusive") {
        format!("{msg}. Wait for Settings → Engines → LSP to finish indexing, then retry")
    } else {
        msg
    };
    coded(code, msg)
}

fn finish_locations(out: crate::lsp::LspToolOutput, roots: &[&Path]) -> ToolCallResult {
    let mut locations = extract_location_items(&out.raw, roots);
    let truncated = locations.len() > MAX_LOCATIONS;
    if truncated {
        locations.truncate(MAX_LOCATIONS);
    }
    let message = if locations.is_empty() {
        "No locations found (language server ready; no definition/reference at this position)."
            .to_string()
    } else if truncated {
        format!("Returning first {MAX_LOCATIONS} locations (more omitted).")
    } else {
        format!("{} location(s).", locations.len())
    };
    with_content_modified(
        crate::tool::outcome::ok_data(serde_json::json!({
            "locations": locations,
            "message": message,
        })),
        out.content_modified,
    )
}

fn finish_hover(out: crate::lsp::LspToolOutput) -> ToolCallResult {
    let markdown = extract_hover_markdown(&out.raw);
    let empty = markdown.trim().is_empty() || markdown == "No hover information available";
    let message = if empty {
        "No hover information available.".to_string()
    } else {
        "Hover information.".to_string()
    };
    let markdown = if empty { String::new() } else { markdown };
    with_content_modified(
        crate::tool::outcome::ok_data(serde_json::json!({
            "markdown": markdown,
            "message": message,
        })),
        out.content_modified,
    )
}

fn finish_diagnostics(out: crate::lsp::LspToolOutput, file_path: &str) -> ToolCallResult {
    let mut items = extract_diagnostic_items(&out.raw, file_path);
    let truncated = items.len() > MAX_DIAGNOSTIC_ITEMS;
    if truncated {
        items.truncate(MAX_DIAGNOSTIC_ITEMS);
    }
    let message = if items.is_empty() {
        "No diagnostics.".to_string()
    } else if truncated {
        format!("Returning first {MAX_DIAGNOSTIC_ITEMS} diagnostics (more omitted).")
    } else {
        format!("{} diagnostic(s).", items.len())
    };
    with_content_modified(
        crate::tool::outcome::ok_data(serde_json::json!({
            "items": items,
            "message": message,
        })),
        out.content_modified,
    )
}

fn finish_symbols(
    out: crate::lsp::LspToolOutput,
    require_path: bool,
    roots: &[&Path],
) -> ToolCallResult {
    let mut symbols = extract_symbol_items(&out.raw, require_path, roots);
    let truncated = symbols.len() > MAX_SYMBOLS;
    if truncated {
        symbols.truncate(MAX_SYMBOLS);
    }
    let message = if symbols.is_empty() {
        "No symbols found.".to_string()
    } else if truncated {
        format!("Returning first {MAX_SYMBOLS} symbols (more omitted).")
    } else {
        format!("{} symbol(s).", symbols.len())
    };
    with_content_modified(
        crate::tool::outcome::ok_data(serde_json::json!({
            "symbols": symbols,
            "message": message,
        })),
        out.content_modified,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_tool_names_are_snake_case_seven() {
        assert_eq!(LSP_CODE_TOOL_NAMES.len(), 7);
        for name in LSP_CODE_TOOL_NAMES {
            assert!(!name.contains("lsp") || *name != "lsp");
            assert!(name.chars().all(|c| c.is_ascii_lowercase() || c == '_'));
        }
    }

    #[test]
    fn all_code_tools_are_code_surface_no_direct_lsp() {
        let engines = WorkspaceEngines::new();
        let tools = all_code_tools(engines, PathBuf::from("."));
        assert_eq!(tools.len(), 7);
        let names: Vec<_> = tools.iter().map(|t| t.name().to_string()).collect();
        assert_eq!(
            names,
            [
                "go_to_definition",
                "find_references",
                "hover",
                "diagnostics",
                "document_symbol",
                "workspace_symbol",
                "go_to_implementation",
            ]
        );
        let surface =
            crate::tool::code_contract::build_surface(&tools).expect("LSP JS contract surface");
        for tool in &tools {
            assert_eq!(tool.surface(), crate::tool::ToolSurface::Code);
            assert_ne!(tool.name(), "lsp");
            assert!(surface.tools.contains_key(tool.name()));
        }
    }

    #[test]
    fn position_schema_requires_path_line_character() {
        let engines = WorkspaceEngines::new();
        let tool = all_code_tools(engines, PathBuf::from("."))
            .into_iter()
            .find(|t| t.name() == "go_to_definition")
            .unwrap();
        let surface = crate::tool::code_contract::build_surface(&[tool.clone()])
            .expect("go_to_definition JS contract");
        let schema = &surface.tools["go_to_definition"].input_schema;
        assert!(schema["properties"].get("character").is_some());
        assert!(schema["properties"].get("path").is_some());
        assert!(schema["properties"].get("file_path").is_none());
        assert!(schema["properties"].get("text").is_none());
        assert!(schema["properties"].get("action").is_none());
        assert!(
            tool.validate_input(&serde_json::json!({
                "path": "src/main.rs",
                "line": 1,
                "character": 1
            }))
            .is_ok()
        );
        assert!(
            tool.validate_input(&serde_json::json!({
                "path": "src/main.rs",
                "line": 1,
                "text": "main"
            }))
            .is_err()
        );
        assert!(
            tool.validate_input(&serde_json::json!({
                "file_path": "src/main.rs",
                "line": 1,
                "character": 1
            }))
            .is_err()
        );
    }

    #[test]
    fn workspace_symbol_schema_is_query_only() {
        let engines = WorkspaceEngines::new();
        let tool = all_code_tools(engines, PathBuf::from("."))
            .into_iter()
            .find(|t| t.name() == "workspace_symbol")
            .unwrap();
        let surface = crate::tool::code_contract::build_surface(&[tool.clone()])
            .expect("workspace_symbol JS contract");
        let schema = &surface.tools["workspace_symbol"].input_schema;
        assert!(schema["properties"].get("query").is_some());
        assert!(schema["properties"].get("path").is_none());
        assert!(schema["properties"].get("file_path").is_none());
        assert!(schema["properties"].get("line").is_none());
        assert!(
            tool.validate_input(&serde_json::json!({ "query": "Foo" }))
                .is_ok()
        );
        assert!(tool.validate_input(&serde_json::json!({})).is_err());
    }

    #[test]
    fn character_out_of_range_is_hard_error() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("lib.rs");
        std::fs::write(&file, "fn main() {}\n").unwrap();
        // "fn main() {}" UTF-16 length 12 → valid character 1..=13
        assert!(validate_position_on_file(&file, 1, 1).is_ok());
        assert!(validate_position_on_file(&file, 1, 13).is_ok());
        assert!(validate_position_on_file(&file, 1, 14).is_err());
        assert!(validate_position_on_file(&file, 2, 1).is_err());
        assert!(validate_position_on_file(&file, 0, 1).is_err());
    }

    #[test]
    fn character_utf16_multibyte() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("lib.rs");
        // `😀` is two UTF-16 units; line "a😀b" → len 4; valid 1..=5
        std::fs::write(&file, "a😀b\n").unwrap();
        assert!(validate_position_on_file(&file, 1, 4).is_ok());
        assert!(validate_position_on_file(&file, 1, 5).is_ok());
        assert!(validate_position_on_file(&file, 1, 6).is_err());
    }

    #[test]
    fn absolute_outside_root_rejected() {
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
    }

    #[test]
    fn directory_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let err = resolve_in_workspace(dir.path(), ".").unwrap_err();
        assert!(err.contains("requires a real source file"), "got: {err}");
    }

    #[test]
    fn finish_locations_empty_resolves_with_empty_array() {
        let out = crate::lsp::LspToolOutput::new("No locations found".into(), Value::Null, false);
        let result = finish_locations(out, &[]);
        assert_eq!(result.level, crate::types::result::ToolSignalLevel::Ok);
        assert!(result.error.is_none());
        let data = result.data.expect("data");
        assert_eq!(data["locations"], serde_json::json!([]));
        assert!(data["message"].as_str().unwrap().contains("No locations"));
    }

    #[test]
    fn index_not_ready_rejects_unavailable_for_symbols_and_diagnostics() {
        // WP-206 ③: documentSymbol / workspaceSymbol / diagnostics share the nav error.
        for method in [
            "textDocument/documentSymbol",
            "workspace/symbol",
            "textDocument/publishDiagnostics",
        ] {
            let err = crate::lsp::server::index_not_ready_error(method);
            let result = map_lsp_query_error(err);
            assert_eq!(
                result.error_code.as_deref(),
                Some("unavailable"),
                "{method}"
            );
            assert!(
                result.data.is_none(),
                "{method}: must not be an empty success"
            );
        }
        // Hub's "server not running" also lands on unavailable.
        let result = map_lsp_query_error("language server is not ready");
        assert_eq!(result.error_code.as_deref(), Some("unavailable"));
    }

    #[test]
    fn finish_locations_relative_inside_absolute_outside() {
        // WP-206 ⑧
        let ws = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let mk = |p: &Path| {
            serde_json::json!({
                "uri": crate::lsp::uri::file_to_uri(p),
                "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 1 } }
            })
        };
        let raw = serde_json::json!([
            mk(&ws.path().join("a").join("b.rs")),
            mk(&other.path().join("c.rs"))
        ]);
        let out = crate::lsp::LspToolOutput::new(String::new(), raw, false);
        let result = finish_locations(out, &[ws.path()]);
        let data = result.data.expect("data");
        assert_eq!(data["locations"][0]["path"], "a/b.rs");
        let outside = data["locations"][1]["path"].as_str().unwrap();
        assert!(
            Path::new(outside).is_absolute() && outside.ends_with("c.rs"),
            "{outside}"
        );
    }

    #[test]
    fn finish_symbols_workspace_paths_relative() {
        let ws = tempfile::tempdir().unwrap();
        let raw = serde_json::json!([{ "name": "f", "kind": 12, "location": {
            "uri": crate::lsp::uri::file_to_uri(&ws.path().join("m.rs")),
            "range": { "start": { "line": 4, "character": 0 }, "end": { "line": 4, "character": 1 } } } }]);
        let out = crate::lsp::LspToolOutput::new(String::new(), raw, false);
        let data = finish_symbols(out, true, &[ws.path()]).data.expect("data");
        assert_eq!(data["symbols"][0]["path"], "m.rs");
        assert_eq!(data["symbols"][0]["line"], 5);
    }
}

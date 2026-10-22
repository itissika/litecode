//! Optional LSP diagnostics feedback for write/edit tools.
//!
//! Only a ready Error diagnostic is a hint. Warming, Failed, timeout,
//! Unavailable, clean publish, and uncovered paths are silence — the Agent
//! must not learn that this path exists unless there is a real diagnostic.
//!
//! Diagnostics always go through [`LspHub`] async APIs (sole exit).

use std::path::PathBuf;

use crate::engines::{EngineState, WorkspaceEngines};
use crate::lsp::LspDiagFeedback;
use crate::tool::outcome::Diagnostic;
use crate::types::ToolCallResult;

const LSP_ERROR_MESSAGE: &str =
    "LSP reported errors in this file. Safe to ignore if intentional — fix when convenient.";

#[cfg(test)]
static LSP_ERROR_HOOK: std::sync::Mutex<Option<(String, String)>> = std::sync::Mutex::new(None);

#[cfg(test)]
pub struct LspErrorHookGuard;

#[cfg(test)]
impl Drop for LspErrorHookGuard {
    fn drop(&mut self) {
        if let Ok(mut hook) = LSP_ERROR_HOOK.lock() {
            *hook = None;
        }
    }
}

/// Test seam: the next write/edit whose path contains `path_needle` records an
/// `lsp_error` diagnostic without a language server.
#[cfg(test)]
pub fn arm_lsp_error_hook(path_needle: &str, block: &str) -> LspErrorHookGuard {
    *LSP_ERROR_HOOK.lock().expect("lsp hook") = Some((path_needle.to_string(), block.to_string()));
    LspErrorHookGuard
}

fn lsp_error_diagnostic(block: &str) -> Diagnostic {
    Diagnostic::warning("lsp_error", LSP_ERROR_MESSAGE)
        .with_details(serde_json::json!({ "text": block }))
}

#[cfg(test)]
fn hooked_lsp_diagnostic(file_path: &str) -> Option<Diagnostic> {
    let hook = LSP_ERROR_HOOK.lock().ok()?;
    let (needle, block) = hook.as_ref()?;
    if file_path.contains(needle) {
        Some(lsp_error_diagnostic(block))
    } else {
        None
    }
}

/// Append file-local Error diagnostics to a successful write/edit result.
/// Any non-Error outcome is returned unchanged.
pub async fn maybe_append_local_lsp_errors(
    engines: Option<&WorkspaceEngines>,
    file_path: &str,
    result: ToolCallResult,
) -> ToolCallResult {
    #[cfg(test)]
    if let Some(diagnostic) = hooked_lsp_diagnostic(file_path) {
        return result.with_diagnostic(diagnostic);
    }
    let Some(engines) = engines else {
        return result;
    };

    let path = {
        let raw = PathBuf::from(file_path);
        if raw.exists() {
            crate::config::path::canon_abs_lossy(&raw)
        } else {
            crate::config::path::strip_verbatim(&raw)
        }
    };

    let hub = engines.lsp_hub();
    if !hub.file_has_lsp_coverage(&path) {
        return result;
    }

    if engines.state("lsp") != Some(EngineState::Warm) {
        return result;
    }

    match hub.file_error_diagnostics_feedback_ex(&path).await {
        LspDiagFeedback::Errors(block) => result.with_diagnostic(lsp_error_diagnostic(&block)),
        LspDiagFeedback::Unavailable(_) | LspDiagFeedback::Silence => result,
    }
}

#[cfg(test)]
mod tests {
    
    

}

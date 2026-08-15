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
use crate::types::ToolCallResult;

const LSP_HINT_PREFIX: &str = "LSP note — ";

/// Append file-local Error diagnostics to a successful write/edit result.
/// Any non-Error outcome is returned unchanged.
pub async fn maybe_append_local_lsp_errors(
    engines: Option<&WorkspaceEngines>,
    file_path: &str,
    result: ToolCallResult,
) -> ToolCallResult {
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
        LspDiagFeedback::Errors(block) => result.with_hint_appendix(
            format!(
                "{LSP_HINT_PREFIX}reported errors in this file. \
                 Safe to ignore if intentional — fix when convenient."
            ),
            Some(block),
        ),
        LspDiagFeedback::Unavailable(_) | LspDiagFeedback::Silence => result,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engines::{EngineState, WorkspaceEngines};

    #[tokio::test]
    async fn no_engines_leaves_content_untouched() {
        let base = ToolCallResult::ok("Created: /tmp/x.rs (10 bytes)");
        let out = maybe_append_local_lsp_errors(None, "/tmp/x.rs", base)
            .await
            .finalize_signals();
        assert_eq!(out.content, "Created: /tmp/x.rs (10 bytes)");
        assert!(!out.content.contains("Hint:"));
    }

    #[tokio::test]
    async fn idle_engines_stay_silent() {
        let engines = WorkspaceEngines::new();
        let base = ToolCallResult::ok("Edited /tmp/x.rs");
        let out = maybe_append_local_lsp_errors(Some(&engines), "/tmp/x.rs", base)
            .await
            .finalize_signals();
        assert_eq!(out.content, "Edited /tmp/x.rs");
        assert!(!out.content.contains("Hint:"));
    }

    #[tokio::test]
    async fn warming_stays_silent() {
        let engines = WorkspaceEngines::new();
        engines.set_state_for_test("lsp", EngineState::Warming);
        engines
            .lsp_hub()
            .set_configured_commands_for_test(&["rust-analyzer".into()]);
        let base = ToolCallResult::ok("Edited /tmp/x.rs");
        let out = maybe_append_local_lsp_errors(Some(&engines), "/tmp/x.rs", base)
            .await
            .finalize_signals();
        assert_eq!(out.content, "Edited /tmp/x.rs");
        assert!(!out.content.contains("Hint:"));
        assert!(!out.content.contains("LSP note"));
    }

    #[tokio::test]
    async fn failed_stays_silent() {
        let engines = WorkspaceEngines::new();
        engines.set_state_for_test("lsp", EngineState::Failed);
        engines.set_last_error_for_test("lsp", "rust-analyzer missing");
        engines
            .lsp_hub()
            .set_configured_commands_for_test(&["rust-analyzer".into()]);
        let base = ToolCallResult::ok("Created: a.rs");
        let out = maybe_append_local_lsp_errors(Some(&engines), "/tmp/a.rs", base)
            .await
            .finalize_signals();
        assert_eq!(out.content, "Created: a.rs");
        assert!(!out.content.contains("Hint:"));
        assert!(!out.content.contains("LSP note"));
        assert!(!out.content.contains("rust-analyzer missing"));
    }

    #[tokio::test]
    async fn uncovered_extension_stays_silent_even_when_warm() {
        let engines = WorkspaceEngines::new();
        engines.set_state_for_test("lsp", EngineState::Warm);
        engines
            .lsp_hub()
            .set_configured_commands_for_test(&["rust-analyzer".into()]);
        let base = ToolCallResult::ok("Updated: notes.md");
        let out = maybe_append_local_lsp_errors(Some(&engines), "/tmp/notes.md", base)
            .await
            .finalize_signals();
        assert_eq!(out.content, "Updated: notes.md");
        assert!(!out.content.contains("Hint:"));
    }
}

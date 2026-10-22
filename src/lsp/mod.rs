//! LSP Hub: single language-server process pool shared by Agent tool and Editor.

pub mod conn;
pub mod deps;
pub mod format;
pub mod hub;
pub mod install;
pub mod paths;
pub mod project_root;
pub mod server;
pub mod server_map;
pub mod status;
pub mod sweep_gate;
pub mod uri;

pub use conn::LspDiagnosticEvent;
pub use conn::LspReply;
pub use hub::{LspDiagFeedback, LspHub, LspToolOutput, SharedLspHub};
pub use server_map::{
    check_workspace_dependencies, command_parts, detect_needed_server_commands,
    program_from_command, server_command_for_ext, server_map,
};
pub use status::{LspInstanceStatus, LspLifecycle};
pub use uri::file_to_uri;

#[cfg(test)]
mod tests {
    use std::path::Path;

    

    use super::*;
    use crate::lsp::format::format_error_diagnostics_block;
    
    
    
    use crate::lsp::uri::uri_to_path;

    #[test]
    fn error_diagnostics_block_only_includes_errors() {
        let text = format_error_diagnostics_block(&serde_json::json!([
            {"severity": 1, "message": "boom", "range": {"start": {"line": 2, "character": 4}}},
            {"severity": 2, "message": "warn", "range": {"start": {"line": 0, "character": 0}}}
        ]))
        .expect("errors present");
        assert!(text.contains("Error: boom"));
        assert!(!text.contains("warn"));
    }

    /// Drive letters are case-insensitive only on Windows; on Unix `/E:` and
    /// `/e:` are distinct directories, so this half is Windows-only.
    #[cfg(windows)]
    #[test]
    fn publish_diagnostics_uri_matches_drive_case() {
        assert!(publish_diagnostics_uri_matches(
            "file:///E:/litecode/src/a.rs",
            "file:///e:/litecode/src/a.rs"
        ));
    }

    /// `Path::starts_with` on `E:\...` only splits into components on
    /// Windows; on Unix the whole string is one component.
    #[cfg(windows)]
    #[test]
    fn normalize_makes_verbatim_path_prefix_compatible() {
        let root = PathBuf::from(r"E:\litecode");
        let verbatim_file = PathBuf::from(r"\\?\E:\litecode\src\agent\core.rs");
        assert!(
            !verbatim_file.starts_with(&root),
            "Windows verbatim vs stripped root must diverge before normalize"
        );
        let normalized = crate::config::path::strip_verbatim(&verbatim_file);
        assert_eq!(normalized, PathBuf::from(r"E:\litecode\src\agent\core.rs"));
        assert!(
            normalized.starts_with(&root),
            "after normalize, hub starts_with must succeed"
        );
    }

    #[test]
    fn uri_to_path_recovers_mangled_verbatim_file_uri() {
        // Platform-independent: the mangled URI is rewritten and parses.
        let mangled = "file:////?/E:/litecode/src/agent/core.rs";
        assert_eq!(
            crate::lsp::uri::normalize_windows_file_uri(mangled),
            "file:///E:/litecode/src/agent/core.rs"
        );
        let path = uri_to_path(mangled).expect("mangled uri should parse after normalize");
        assert_eq!(
            Some(path),
            uri_to_path("file:///E:/litecode/src/agent/core.rs"),
            "mangled and clean URIs map to the same path"
        );
        // The concrete `E:\` form is Windows path semantics.
        #[cfg(windows)]
        assert_eq!(
            uri_to_path(mangled).unwrap(),
            PathBuf::from(r"E:\litecode\src\agent\core.rs")
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn restart_server_rejects_inactive_hub() {
        let hub = std::sync::Arc::new(LspHub::new());
        let err = hub
            .restart_server("rust-analyzer", Path::new("/tmp/proj"))
            .await
            .expect_err("inactive hub");
        let msg = err.to_string();
        assert!(
            msg.contains("not active"),
            "expected inactive error, got: {msg}"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn restart_server_rejects_unconfigured_command() {
        let hub = std::sync::Arc::new(LspHub::new());
        hub.set_configured_commands_for_test(&["rust-analyzer".into()]);
        let err = hub
            .restart_server("gopls", Path::new("/tmp/proj"))
            .await
            .expect_err("unconfigured");
        let msg = err.to_string();
        assert!(
            msg.contains("not configured"),
            "expected not configured, got: {msg}"
        );
    }

    // ---- WP-167: deferred availability probe + sweep gate ----



}

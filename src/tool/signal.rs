//! Stable tool signal grammar: Error / Warning / Hint.
//!
//! Tools return structured [`ToolCallResult`] fields; [`compose`] builds the
//! wire text once. Do not hand-prefix `Error:` / `Warning:` / `Hint:` in tool
//! bodies.
//!
//! **Hint is reserved for LSP success feedback.** Other guidance belongs in
//! Error/Warning/Ok content — not a second signal.

use crate::types::{ToolCallResult, ToolSignalLevel};

/// Build the agent/UI-visible text from structured signal fields.
pub fn compose(level: ToolSignalLevel, content: &str, hint: Option<&str>) -> String {
    compose_full(level, content, hint, None, None)
}

/// Full compose: optional lead body + warning status + independent Hint block.
///
/// When `warning_status` is set with [`ToolSignalLevel::Warning`], `content` is the
/// lead success body and `warning_status` is the Warning line. Hint (LSP) is always
/// a separate block — never glued onto Error/Warning as `. Hint:`.
/// `appendix` follows the Hint line (e.g. diagnostics).
pub fn compose_full(
    level: ToolSignalLevel,
    content: &str,
    hint: Option<&str>,
    warning_status: Option<&str>,
    appendix: Option<&str>,
) -> String {
    let mut s = match level {
        ToolSignalLevel::Error => format!("Error: {content}"),
        ToolSignalLevel::Warning => {
            if let Some(status) = warning_status.filter(|s| !s.is_empty()) {
                let mut s = String::new();
                if !content.is_empty() {
                    s.push_str(content);
                    s.push_str("\n\n");
                }
                s.push_str(&format!("Warning: {status}"));
                s
            } else {
                format!("Warning: {content}")
            }
        }
        ToolSignalLevel::Ok => content.to_string(),
    };

    if let Some(h) = hint.filter(|h| !h.is_empty()) {
        if s.is_empty() {
            s = format!("Hint: {h}");
        } else {
            s.push_str("\n\n");
            s.push_str(&format!("Hint: {h}"));
        }
    }
    if let Some(a) = appendix.filter(|a| !a.is_empty()) {
        s.push('\n');
        s.push_str(a);
    }
    s
}

impl ToolCallResult {
    /// Compose `level` / `hint` / optional warning appendix into `content` once.
    pub fn finalize_signals(mut self) -> Self {
        if self.composed {
            return self;
        }
        self.content = compose_full(
            self.level,
            &self.content,
            self.hint.as_deref(),
            self.warning_status.as_deref(),
            self.appendix.as_deref(),
        );
        self.hint = None;
        self.warning_status = None;
        self.appendix = None;
        self.composed = true;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_is_prefix_only() {
        let t = ToolCallResult::error("missing regex").finalize_signals();
        assert_eq!(t.content, "Error: missing regex");
    }

    #[test]
    fn warning_standalone() {
        let t = ToolCallResult::warning("exit_code: 1").finalize_signals();
        assert_eq!(t.content, "Warning: exit_code: 1");
    }

    #[test]
    fn warning_after_body_hint_stays_independent() {
        let t = ToolCallResult::ok("Created: a.rs")
            .with_hint_appendix("LSP note — still warming; diagnostics skipped", None)
            .with_warning("you are writing outside the workspace (/tmp/a.rs)");
        let out = t.finalize_signals();
        assert!(out.content.starts_with("Created: a.rs\n\nWarning: "));
        assert!(out.content.contains("\n\nHint: LSP note — still warming"));
        let warn_at = out.content.find("Warning:").unwrap();
        let hint_at = out.content.find("Hint:").unwrap();
        assert!(
            warn_at < hint_at,
            "Hint must follow Warning, got {}",
            out.content
        );
    }

    #[test]
    fn ok_with_hint_is_separate_block() {
        let t = ToolCallResult::ok("Edited a.rs").with_hint_appendix("LSP note — errors", None);
        let out = t.finalize_signals();
        assert_eq!(out.content, "Edited a.rs\n\nHint: LSP note — errors");
    }

    #[test]
    fn ok_hint_appendix_after_body() {
        let t = ToolCallResult::ok("Created: a.rs").with_hint_appendix(
            "LSP reported errors (ignore if intentional)",
            Some("error[E0001]: mock".into()),
        );
        let out = t.finalize_signals();
        assert!(out.content.starts_with("Created: a.rs\n\nHint: "));
        assert!(out.content.contains("mock"));
    }

    #[test]
    fn finalize_is_idempotent() {
        let t = ToolCallResult::error("boom").finalize_signals();
        let again = t.clone().finalize_signals();
        assert_eq!(t.content, again.content);
        assert_eq!(t.content, "Error: boom");
    }
}

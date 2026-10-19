use super::media::ToolOutputPart;

/// Wire-facing severity before [`crate::tool::signal`] compose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ToolSignalLevel {
    #[default]
    Ok,
    Warning,
    Error,
}

/// Stable tool error categories exposed to internal consumers and the future
/// code-tool runtime. `content` remains the model/UI text projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolErrorKind {
    InvalidInput,
    NotFound,
    PermissionDenied,
    Conflict,
    Unavailable,
    Timeout,
    Cancelled,
    Io,
    OutputLimit,
    InvalidOutput,
    Internal,
}

impl ToolErrorKind {
    pub const ALL: [Self; 11] = [
        Self::InvalidInput,
        Self::NotFound,
        Self::PermissionDenied,
        Self::Conflict,
        Self::Unavailable,
        Self::Timeout,
        Self::Cancelled,
        Self::Io,
        Self::OutputLimit,
        Self::InvalidOutput,
        Self::Internal,
    ];
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ToolError {
    pub kind: ToolErrorKind,
    pub message: String,
    pub retryable: bool,
}

#[derive(Debug, Clone)]
pub struct ToolCallResult {
    /// Primary body / status text **without** `Error:` / `Warning:` prefixes.
    /// After [`Self::finalize_signals`], holds the composed wire text.
    pub content: String,
    /// Structured domain result. Core bundle tools must populate this; it is
    /// the contract consumed by code orchestration and contract tests.
    pub data: Option<serde_json::Value>,
    /// Stable error classification. `content` remains the rendered text.
    pub error: Option<ToolError>,
    pub parts: Vec<ToolOutputPart>,
    pub metadata: Option<serde_json::Value>,
    pub level: ToolSignalLevel,
    /// LSP success note without the `Hint:` prefix. Other tools put guidance in `content`.
    pub hint: Option<String>,
    /// When set with [`ToolSignalLevel::Warning`], `content` is the lead success
    /// body and this is the Warning status line.
    pub warning_status: Option<String>,
    /// Extra block after the Hint/Warning/Error status line (e.g. LSP diagnostics).
    pub appendix: Option<String>,
    /// Whether [`Self::finalize_signals`] has already composed wire text.
    pub composed: bool,
}

impl ToolCallResult {
    pub fn ok(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            data: None,
            error: None,
            parts: Vec::new(),
            metadata: None,
            level: ToolSignalLevel::Ok,
            hint: None,
            warning_status: None,
            appendix: None,
            composed: false,
        }
    }

    pub fn ok_with_hint(content: impl Into<String>, hint: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            data: None,
            error: None,
            parts: Vec::new(),
            metadata: None,
            level: ToolSignalLevel::Ok,
            hint: Some(hint.into()),
            warning_status: None,
            appendix: None,
            composed: false,
        }
    }

    pub fn ok_with_metadata(content: impl Into<String>, metadata: serde_json::Value) -> Self {
        Self {
            content: content.into(),
            data: None,
            error: None,
            parts: Vec::new(),
            metadata: Some(metadata),
            level: ToolSignalLevel::Ok,
            hint: None,
            warning_status: None,
            appendix: None,
            composed: false,
        }
    }

    pub fn ok_with_parts(content: impl Into<String>, parts: Vec<ToolOutputPart>) -> Self {
        Self {
            content: content.into(),
            data: None,
            error: None,
            parts,
            metadata: None,
            level: ToolSignalLevel::Ok,
            hint: None,
            warning_status: None,
            appendix: None,
            composed: false,
        }
    }

    /// Attach the structured, code-facing result. Core bundle tools must use
    /// this (or populate the field directly through a typed outcome).
    pub fn with_data(mut self, data: serde_json::Value) -> Self {
        self.data = Some(data);
        self
    }

    /// Mark this result with a stable error category while keeping `content`
    /// as the rendered model/UI text.
    pub fn with_error_kind(
        mut self,
        kind: ToolErrorKind,
        message: impl Into<String>,
        retryable: bool,
    ) -> Self {
        self.error = Some(ToolError {
            kind,
            message: message.into(),
            retryable,
        });
        self
    }

    pub fn error(msg: impl Into<String>) -> Self {
        Self::error_with_kind(ToolErrorKind::Internal, msg, false)
    }

    pub fn error_with_kind(kind: ToolErrorKind, msg: impl Into<String>, retryable: bool) -> Self {
        let msg = msg.into();
        Self {
            content: msg.clone(),
            data: None,
            error: Some(ToolError {
                kind,
                message: msg,
                retryable,
            }),
            parts: Vec::new(),
            metadata: None,
            level: ToolSignalLevel::Error,
            hint: None,
            warning_status: None,
            appendix: None,
            composed: false,
        }
    }

    pub fn error_with_hint(msg: impl Into<String>, hint: impl Into<String>) -> Self {
        let msg = msg.into();
        Self {
            content: msg.clone(),
            data: None,
            error: Some(ToolError {
                kind: ToolErrorKind::Internal,
                message: msg,
                retryable: false,
            }),
            parts: Vec::new(),
            metadata: None,
            level: ToolSignalLevel::Error,
            hint: Some(hint.into()),
            warning_status: None,
            appendix: None,
            composed: false,
        }
    }

    pub fn warning(msg: impl Into<String>) -> Self {
        Self {
            content: msg.into(),
            data: None,
            error: None,
            parts: Vec::new(),
            metadata: None,
            level: ToolSignalLevel::Warning,
            hint: None,
            warning_status: None,
            appendix: None,
            composed: false,
        }
    }

    pub fn warning_with_hint(msg: impl Into<String>, hint: impl Into<String>) -> Self {
        Self {
            content: msg.into(),
            data: None,
            error: None,
            parts: Vec::new(),
            metadata: None,
            level: ToolSignalLevel::Warning,
            hint: Some(hint.into()),
            warning_status: None,
            appendix: None,
            composed: false,
        }
    }

    /// Half-success: keep `self.content` as lead body, attach a Warning status.
    /// Does not touch an existing LSP [`Self::hint`] / appendix.
    pub fn with_warning(mut self, warning_status: impl Into<String>) -> Self {
        self.level = ToolSignalLevel::Warning;
        self.warning_status = Some(warning_status.into());
        self.composed = false;
        self
    }

    /// Half-success with a multi-line appendix under the Warning line.
    /// Does not touch an existing LSP [`Self::hint`].
    pub fn with_warning_block(
        mut self,
        warning_status: impl Into<String>,
        appendix: impl Into<String>,
    ) -> Self {
        self.level = ToolSignalLevel::Warning;
        self.warning_status = Some(warning_status.into());
        self.appendix = Some(appendix.into());
        self.composed = false;
        self
    }

    /// LSP success Hint. Does not change `level` — Warning/Ok stay as-is so a
    /// path-risk Warning and an LSP note can coexist.
    pub fn with_hint_appendix(mut self, hint: impl Into<String>, appendix: Option<String>) -> Self {
        self.hint = Some(hint.into());
        self.appendix = appendix;
        self.composed = false;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_survives_signal_finalization() {
        let result = ToolCallResult::ok("done")
            .with_data(serde_json::json!({"items": [1, 2, 3]}))
            .finalize_signals();
        assert_eq!(result.content, "done");
        assert_eq!(result.data, Some(serde_json::json!({"items": [1, 2, 3]})));
        assert!(result.error.is_none());
    }

    #[test]
    fn stable_error_kind_survives_finalization() {
        let result =
            ToolCallResult::error_with_kind(ToolErrorKind::InvalidInput, "bad range", false)
                .finalize_signals();
        assert_eq!(result.content, "Error: bad range");
        let error = result.error.expect("typed error");
        assert_eq!(error.kind, ToolErrorKind::InvalidInput);
        assert_eq!(error.message, "bad range");
        assert!(!error.retryable);
    }

    #[test]
    fn warning_is_not_an_error() {
        let result = ToolCallResult::warning("partial").finalize_signals();
        assert_eq!(result.content, "Warning: partial");
        assert!(result.error.is_none());
    }
}

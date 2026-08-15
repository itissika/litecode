use super::media::ToolOutputPart;

/// Wire-facing severity before [`crate::tool::signal`] compose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ToolSignalLevel {
    #[default]
    Ok,
    Warning,
    Error,
}

#[derive(Debug, Clone)]
pub struct ToolCallResult {
    /// Primary body / status text **without** `Error:` / `Warning:` prefixes.
    /// After [`Self::finalize_signals`], holds the composed wire text.
    pub content: String,
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
            parts,
            metadata: None,
            level: ToolSignalLevel::Ok,
            hint: None,
            warning_status: None,
            appendix: None,
            composed: false,
        }
    }

    pub fn error(msg: impl Into<String>) -> Self {
        Self {
            content: msg.into(),
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
        Self {
            content: msg.into(),
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

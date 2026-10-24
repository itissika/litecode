use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactionFailureKind {
    SummaryFinish,
    Quality,
    Transport,
}

#[derive(Debug, Error)]
pub enum LitecodeError {
    #[error("git error: {0}")]
    Git(#[from] git2::Error),
    #[error("session not found: {0}")]
    SessionNotFound(String),

    #[error("tool not found: {0}")]
    ToolNotFound(String),

    #[error("config error: {0}")]
    Config(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("tool execution error: {0}")]
    ToolExecution(String),

    #[error("media blob unavailable: {0}")]
    MediaBlobMissing(String),

    #[error("hook execution error: {0}")]
    HookExecution(String),

    #[error("token budget exceeded")]
    TokenBudgetExceeded,

    #[error("max steps reached")]
    MaxStepsReached,

    #[error("invalid revert anchor: {0}")]
    InvalidRevertAnchor(String),

    #[error("invalid session event: {0}")]
    InvalidSessionEvent(String),

    #[error("agent already running")]
    AgentAlreadyRunning,

    #[error("compaction failed: empty summary")]
    CompactionFailed,

    #[error("compaction failed ({kind:?}): {detail}")]
    CompactionFailure {
        kind: CompactionFailureKind,
        detail: String,
    },

    #[error("nothing to compact")]
    NothingToCompact,

    #[error("llm error: {0}")]
    Llm(String),

    #[error("llm stream interrupted: {message}")]
    LlmStreamInterrupted {
        message: String,
        partial: Vec<crate::types::Item>,
    },

    #[error("canceled")]
    Canceled,

    #[error("session conflict: expected revision {expected}, actual {actual}")]
    SessionConflict { expected: u64, actual: u64 },

    #[error("session data is closed")]
    SessionDataClosed,

    #[error("session data writer queue is full")]
    SessionBackpressure,

    #[error("session storage error: {0}")]
    SessionStorage(String),

    /// A retrieval lane could not answer the question that was put to it.
    ///
    /// Deliberately its own variant and not a flavour of storage error, because
    /// the two call for opposite responses. A storage error says the question was
    /// asked and the answer is unavailable. This says the question was **never
    /// asked** — the index behind the lane was missing, mid-build, or broken. The
    /// one thing a caller must not do is read that as "the corpus has no such
    /// row": a search that could not run looks exactly like a search that found
    /// nothing, and a caller acts on the difference.
    #[error("retrieval lane did not answer: {0}")]
    IndexNotReady(String),

    #[error("{0}")]
    Anyhow(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, LitecodeError>;

impl LitecodeError {
    /// Structured compaction boundary: summary finish/quality vs transport.
    pub fn compaction_failure_kind(&self) -> Option<CompactionFailureKind> {
        match self {
            Self::CompactionFailure { kind, .. } => Some(*kind),
            _ => None,
        }
    }

    pub fn is_compaction_failure(&self) -> bool {
        self.compaction_failure_kind().is_some() || matches!(self, Self::CompactionFailed)
    }
}

/// Confirmed provider context-window overflow, not a generic 400/transport
/// failure. Keep this classification deliberately narrow: only retry when the
/// provider explicitly says the prompt exceeded the context window.
pub fn is_context_overflow_error(error: &LitecodeError) -> bool {
    let text = match error {
        LitecodeError::Llm(message) => message,
        LitecodeError::LlmStreamInterrupted { message, .. } => message,
        LitecodeError::CompactionFailure { detail, .. } => detail,
        other => {
            let rendered = other.to_string();
            return contains_context_overflow_marker(&rendered);
        }
    };
    contains_context_overflow_marker(text)
}

fn contains_context_overflow_marker(text: &str) -> bool {
    let text = text.to_ascii_lowercase();
    [
        "context_length_exceeded",
        "context_window_exceeded",
        "context length exceeded",
        "context window exceeded",
        "maximum context length",
        "max context length",
        "maximum context window",
        "prompt is too long",
        "prompt too long",
        "input is too long",
        "exceeds the model's context",
        "exceeds the context window",
    ]
    .iter()
    .any(|needle| text.contains(needle))
}

#[cfg(test)]
mod tests {}

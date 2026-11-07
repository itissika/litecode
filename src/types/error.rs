use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactionFailureKind {
    SummaryFinish,
    Quality,
    Transport,
}

/// Stable classification for failures at the session-data write boundary.
///
/// Callers should use this instead of matching error strings. Storage is the
/// only class whose retryability depends on the underlying cause.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionWriteErrorKind {
    Conflict,
    Backpressure,
    Timeout,
    Closed,
    Storage,
}

#[derive(Debug, Error)]
pub enum LitecodeError {
    #[error("git error: {0}")]
    Git(#[from] git2::Error),
    #[error("session not found: {0}")]
    SessionNotFound(String),

    #[error("bot '{0}' owns this session")]
    BotOwnedSession(String),

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

    /// The commit existed but the pipeline could not deliver a verified
    /// surface. This is not success; the session gets the next read and stops
    /// the turn only if that read also fails.
    #[error("compaction could not deliver a verified surface: {0}")]
    CompactionUnverified(String),

    #[error("compaction failed ({kind:?}): {detail}")]
    CompactionFailure {
        kind: CompactionFailureKind,
        detail: String,
    },

    #[error("nothing to compact")]
    NothingToCompact,

    /// The durable prefix changed under a frozen compaction plan (another
    /// checkpoint landed, a cut was deleted, or the shadow start moved).
    /// This is a surface race, not a user cancel.
    #[error("compaction surface changed: {0}")]
    CompactionSurfaceChanged(String),

    /// Even after dropping the oldest history the summary request cannot fit
    /// the model's physical prompt limit.
    #[error("compaction input too large: {0}")]
    CompactionInputTooLarge(String),

    /// After compaction the request still sits above the product/physical hard
    /// line. Sending it would exceed the selected window or prompt ceiling.
    #[error(
        "context is still above the hard limit after compaction (used {used}, limit {limit}); switch to a larger model, revert, or start a new session"
    )]
    ContextStillOverLimit { used: usize, limit: usize },

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

    /// A write waited past its queue deadline. The command may not have run;
    /// if it did run, its operation id still makes a retry idempotent.
    #[error("session data write timed out")]
    SessionDataTimeout,

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
    /// Classify an error that came from the session-data write boundary.
    /// Returns None for errors that are not writer failures.
    pub fn session_write_error_kind(&self) -> Option<SessionWriteErrorKind> {
        match self {
            Self::SessionConflict { .. } => Some(SessionWriteErrorKind::Conflict),
            Self::SessionBackpressure => Some(SessionWriteErrorKind::Backpressure),
            Self::SessionDataTimeout => Some(SessionWriteErrorKind::Timeout),
            Self::SessionDataClosed => Some(SessionWriteErrorKind::Closed),
            Self::SessionStorage(_) | Self::Sqlite(_) => Some(SessionWriteErrorKind::Storage),
            _ => None,
        }
    }

    /// Whether a session-data write may be retried without changing its
    /// meaning. Conflicts, backpressure, and queued-timeout are safe to retry;
    /// closed is terminal; storage is retryable only for SQLite contention.
    pub fn is_retryable_session_write(&self) -> bool {
        match self {
            Self::SessionConflict { .. } | Self::SessionBackpressure | Self::SessionDataTimeout => {
                true
            }
            Self::Sqlite(error) => matches!(
                error.sqlite_error_code(),
                Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked)
            ),
            Self::SessionStorage(message) => {
                let lower = message.to_ascii_lowercase();
                lower.contains("locked") || lower.contains("busy")
            }
            _ => false,
        }
    }

    /// Structured compaction boundary: summary finish/quality vs transport.
    pub fn compaction_failure_kind(&self) -> Option<CompactionFailureKind> {
        match self {
            Self::CompactionFailure { kind, .. } => Some(*kind),
            _ => None,
        }
    }

    pub fn is_compaction_failure(&self) -> bool {
        self.compaction_failure_kind().is_some()
            || matches!(self, Self::CompactionFailed | Self::CompactionUnverified(_))
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
mod tests {
    use super::*;

    #[test]
    fn session_write_errors_have_stable_classes_and_retry_flags() {
        let conflict = LitecodeError::SessionConflict {
            expected: 1,
            actual: 2,
        };
        assert_eq!(
            conflict.session_write_error_kind(),
            Some(SessionWriteErrorKind::Conflict)
        );
        assert!(conflict.is_retryable_session_write());

        let backpressure = LitecodeError::SessionBackpressure;
        assert_eq!(
            backpressure.session_write_error_kind(),
            Some(SessionWriteErrorKind::Backpressure)
        );
        assert!(backpressure.is_retryable_session_write());

        let timeout = LitecodeError::SessionDataTimeout;
        assert_eq!(
            timeout.session_write_error_kind(),
            Some(SessionWriteErrorKind::Timeout)
        );
        assert!(timeout.is_retryable_session_write());

        let closed = LitecodeError::SessionDataClosed;
        assert_eq!(
            closed.session_write_error_kind(),
            Some(SessionWriteErrorKind::Closed)
        );
        assert!(!closed.is_retryable_session_write());
    }

    #[test]
    fn storage_retryability_requires_contention_evidence() {
        let locked = LitecodeError::SessionStorage("database is locked".into());
        assert_eq!(
            locked.session_write_error_kind(),
            Some(SessionWriteErrorKind::Storage)
        );
        assert!(locked.is_retryable_session_write());

        let busy = LitecodeError::SessionStorage("writer busy".into());
        assert!(busy.is_retryable_session_write());

        let disk_full = LitecodeError::SessionStorage("disk full".into());
        assert_eq!(
            disk_full.session_write_error_kind(),
            Some(SessionWriteErrorKind::Storage)
        );
        assert!(!disk_full.is_retryable_session_write());
    }

    #[test]
    fn non_writer_errors_are_not_classified() {
        let error = LitecodeError::InvalidSessionEvent("bad row".into());
        assert_eq!(error.session_write_error_kind(), None);
        assert!(!error.is_retryable_session_write());
    }
}

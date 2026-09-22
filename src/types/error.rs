use thiserror::Error;

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

    #[error("compaction failed after 3 attempts")]
    CompactionFailed,

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

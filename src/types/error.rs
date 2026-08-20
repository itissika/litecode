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

    #[error("canceled")]
    Canceled,

    #[error("{0}")]
    Anyhow(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, LitecodeError>;

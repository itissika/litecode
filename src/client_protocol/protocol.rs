use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

pub use crate::runtime::observer::{
    CompactionFailKind, CompactionKind, CompactionStage, CompactionTrigger, TurnEndReason,
    TurnTokenStats,
};
use crate::session::task_state::TodoItem;

pub mod methods {
    pub const AGENT_RUN: &str = "agent/run";
    pub const AGENT_CANCEL: &str = "agent/cancel";
    pub const AGENT_PERMISSION: &str = "agent/permission";
    pub const SESSION_SNAPSHOT: &str = "session/snapshot";
    pub const SESSION_SUBSCRIBE: &str = "session/subscribe";
    pub const SESSION_UNSUBSCRIBE: &str = "session/unsubscribe";
    pub const BUFFER_LOAD: &str = "buffer/load";
    pub const BUFFER_ITEM: &str = "buffer/item";
    /// Parent session: child session created for a `subagent_launch` call_id.
    pub const AGENT_SUBAGENT_BOUND: &str = "agent/subagent_bound";
    pub const SESSION_NEW: &str = "session/new";
    pub const SESSION_DELETE: &str = "session/delete";
    pub const SESSION_LIST: &str = "session/list";
    pub const SESSION_COMPACT: &str = "session/compact";
    pub const SESSION_COMPACT_LIFECYCLE: &str = "session/compact_lifecycle";
    pub const SESSION_REVERT_TO_USER_ANCHOR: &str = "session/revert-to-user-anchor";
    pub const SESSION_REVERT_FILES: &str = "session/revert-files";
    pub const AGENT_SET_PRIMARY: &str = "agent/set-primary";
    pub const AGENT_SET_MODEL: &str = "agent/set-model";
    pub const AGENT_SET_THINKING_TIER: &str = "agent/set-thinking-tier";
    pub const AGENT_SET_CONTEXT_MODE: &str = "agent/set-context-mode";
    pub const LSP_REQUEST: &str = "lsp/request";
    pub const LSP_DIAGNOSTICS: &str = "lsp/diagnostics";
    pub const TERMINAL_CREATE: &str = "terminal/create";
    pub const TERMINAL_WRITE: &str = "terminal/write";
    pub const TERMINAL_RESIZE: &str = "terminal/resize";
    pub const TERMINAL_CLOSE: &str = "terminal/close";
    pub const BASH_JOBS: &str = "bash/jobs";
    pub const BASH_TAIL: &str = "bash/tail";
    pub const BASH_KILL: &str = "bash/kill";
    pub const SUBSCRIBE_LOGS: &str = "subscribe_logs";
    pub const UNSUBSCRIBE_LOGS: &str = "unsubscribe_logs";
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    LlmHttp,
    LlmParse,
    MaxSteps,
    CompactionFailed,
    CompactionCircuitOpen,
    PermissionDenied,
    ToolPanic,
    ToolValidation,
    Cancelled,
    SessionNotFound,
    InvalidRequest,
    AgentAlreadyRunning,
    InvalidRevertAnchor,
    SnapshotUnavailable,
    NothingToRevert,
    NothingToCompact,
    Internal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructuredError {
    pub code: ErrorCode,
    pub message: String,
}

/// Wire-serializable turn phase (L2 projection of L1 `runtime::observer::TurnPhase`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WireTurnPhase {
    Idle,
    Starting,
    Compacting,
    CallingLlm,
    Streaming,
    ExecutingTools,
    AwaitingPermission {
        tool: String,
        rule_id: String,
        summary: String,
    },
    Cancelling,
    Finalizing,
    Failed {
        code: ErrorCode,
    },
}

/// Model summary sent in handshake for the model switcher UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub api_model_id: String,
    pub label: String,
    pub context_window: usize,
    /// Adapter id for the model's provider (e.g. deepseek_responses) — session UI ecosystem.
    #[serde(default)]
    pub adapter_id: String,
}

/// Wire handshake payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerHello {
    pub version: String,
    pub session_id: String,
    pub project: String,
    pub settings_revision: u64,
    pub active_primary: String,
    pub primary_agents: Vec<PrimaryAgentInfo>,
    /// Provider ecosystem for chat-bar controls (`deepseek`, `openai`, …).
    #[serde(default = "default_llm_ecosystem")]
    pub llm_ecosystem: String,
    /// Available models for the model switcher.
    #[serde(default)]
    pub models: Vec<ModelInfo>,
}

fn default_llm_ecosystem() -> String {
    "openai".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrimaryAgentInfo {
    pub id: String,
    pub description: String,
}

/// Settings changed notification (no secrets).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingsChanged {
    pub revision: u64,
    pub docs: Vec<crate::config::DocId>,
    pub summary: crate::config::SettingsSummary,
}

/// Periodic process memory sample for the Web status bar.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerStats {
    /// Total RSS: core + embed + lsp child processes (KB).
    pub rss_kb: Option<u64>,
    pub core_rss_kb: Option<u64>,
    pub embed_rss_kb: Option<u64>,
    pub lsp_rss_kb: Option<u64>,
    pub ts_ms: i64,
}

/// Live tracing event forwarded to subscribed Web clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogLine {
    pub ts_ms: i64,
    pub level: String,
    pub target: String,
    pub message: String,
}

/// Element of SessionList.sessions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: String,
    pub project: String,
    pub updated_at: i64,
    pub preview: String,
    pub running: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn: Option<TurnSnapshot>,
    pub agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    pub api_model_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    /// Set when this session is a persisted subagent child of another session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
    /// Parent `function_call.call_id` that launched this child session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_call_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionList {
    pub sessions: Vec<SessionInfo>,
}

/// Workspace file change notification payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceChanged {
    pub paths: Vec<String>,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspWireError {
    pub code: i64,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspResult {
    pub id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<LspWireError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspNotification {
    pub method: String,
    pub params: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BufferState {
    /// Highest persisted log seq, or `-1` when the log is empty.
    pub last_seq: i64,
    /// Next seq the allocator will assign (`last_seq + 1`, or `0` if empty).
    pub next_seq: u64,
    pub revision: u64,
}

/// Durable session metadata. This deliberately excludes process-local turn,
/// token, terminal, and catalog projection state.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionMetaWire {
    pub id: String,
    pub project: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub parent_session_id: Option<String>,
    pub parent_call_id: Option<String>,
    pub subagent_depth: u32,
    pub agent_id: String,
    pub model_id: Option<String>,
    pub thinking_tier: String,
    pub context_mode: String,
    pub compacted_seq: Option<u64>,
    pub spine_from: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub todos: Vec<serde_json::Value>,
    pub plan_slug: Option<String>,
    pub preview: String,
}

/// One persisted log row as shipped on `buffer/item` and `buffer/load`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireBufferEvent {
    pub seq: crate::session::event::Seq,
    #[serde(rename = "kind")]
    pub event_type: crate::session::event::EventType,
    /// Kind-specific durable body. Consumers must switch on `type`; this is
    /// the authoritative session product payload.
    pub body: serde_json::Value,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cites: Vec<crate::session::event::Seq>,
    #[serde(default)]
    pub state: crate::session::model::LogState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surface_op: Option<crate::session::surface::SurfaceOp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_session_id: Option<String>,
}

/// RPC result for `buffer/load`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BufferLoadResult {
    pub session_id: String,
    pub from_seq: crate::session::event::Seq,
    pub to_seq: crate::session::event::Seq,
    pub events: Vec<WireBufferEvent>,
    #[serde(default)]
    pub subagent_bindings: HashMap<String, String>,
    pub user_detail_before: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnSnapshot {
    pub turn_id: String,
    pub phase: WireTurnPhase,
    pub step: u64,
    pub step_max: u32,
    pub started_at_ms: i64,
    #[serde(default)]
    pub awaiting_permission: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSnapshot {
    /// New session contract. Legacy flattened fields remain only while all
    /// snapshot producers are migrated in this release.
    #[serde(default)]
    pub meta: SessionMetaWire,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live: Option<TurnSnapshot>,
    pub session_id: String,
    pub project: String,
    pub agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    pub api_model_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    pub buffer: BufferState,
    pub turn: Option<TurnSnapshot>,
    #[serde(default)]
    pub context_window: usize,
    /// Local estimate of the current model working set. This is the authority
    /// for occupancy UI and the manual-compaction product gate.
    #[serde(default)]
    pub context_tokens_estimate: usize,
    /// Item-text buckets aligned with the last prepared request when present.
    #[serde(default)]
    pub context_token_breakdown: crate::session::estimate::ItemTokenBreakdown,
    #[serde(default)]
    pub compact_eligible: bool,
    /// Standalone manual compaction owns the session (not part of `turn`).
    #[serde(default)]
    pub compacting: bool,
    /// Last-known provider usage for this session (ring / cache hit rate).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_turn_token_stats: Option<TurnTokenStats>,
    /// Session-total provider usage (Σ every request's usage; whole-session
    /// token-weighted cache hit rate — industry aggregate, e.g. LiteLLM).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cumulative_token_stats: Option<TurnTokenStats>,
    #[serde(default = "default_thinking_tier")]
    pub thinking_tier: String,
    #[serde(default = "default_context_mode")]
    pub context_mode: String,
    /// Highest user-detail anchor `k` whose file patch is nonempty.
    /// FE shows "Revert files" on user messages with `userAnchorK <= this`.
    /// `None` = no file-level revert available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_file_revert_k: Option<i64>,
    /// Running agent bash jobs and wait_shell waiters for this session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bash: Option<crate::terminal::BashJobsSnapshot>,
    /// Session-scoped todo list (reconnect / snapshot hydrate). Compact does
    /// not rewrite this column; the panel must not wait for the next turn event.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub todos: Vec<TodoItem>,
    /// Active workspace-relative plan file for the session, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_plan_path: Option<String>,
}

fn default_thinking_tier() -> String {
    "medium".into()
}

fn default_context_mode() -> String {
    "standard".into()
}

/// Session binding + effective projection for wire snapshots / list / ops.
///
/// Built from sticky `agent_id`/`model_id` plus catalog lookup. Empty `model_id`
/// yields empty effective fields (UI empty); turn resolve hard-fails separately.
#[derive(Debug, Clone, Default)]
pub struct SessionBindingProjection {
    pub agent_id: String,
    pub model_id: Option<String>,
    pub api_model_id: String,
    pub label: String,
    pub context_window: usize,
    pub thinking_tier: String,
    pub context_mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnStarted {
    pub session_id: String,
    pub turn_id: String,
    pub input: String,
    pub step_max: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnEventEnvelope {
    pub session_id: String,
    pub turn_id: String,
    pub event: WireEvent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnFinished {
    pub session_id: String,
    pub turn_id: String,
    pub reason: TurnEndReason,
    pub final_text: Option<String>,
    pub error: Option<StructuredError>,
    pub snapshot: SessionSnapshot,
    #[serde(default)]
    pub turn_token_stats: Option<TurnTokenStats>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    Start,
    NewSession,
    DeleteSession,
    ListSessions,
    CompactSession,
    RevertToUserAnchor,
    RevertFiles,
    SetActivePrimary,
    SetModel,
    SetThinkingTier,
    SetContextMode,
    LspRequest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationResult {
    pub op: OperationKind,
    pub ok: bool,
    pub error: Option<StructuredError>,
    pub snapshot: SessionSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionRequest {
    pub session_id: String,
    pub turn_id: String,
    pub request_id: String,
    pub tool: String,
    pub rule_id: String,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WireEvent {
    /// Authority Responses stream event (passthrough projection of `InternalEvent::StreamEvent`).
    StreamEvent {
        event: crate::types::StreamEvents,
    },
    TodoProgress {
        pending: usize,
        in_progress: usize,
        completed: usize,
        items: Vec<TodoItem>,
    },
    PlanChanged {
        active_plan_path: Option<String>,
    },
    PhaseChanged {
        phase: WireTurnPhase,
        step: u64,
    },
    StepStarted {
        step: u64,
        step_max: u32,
    },
    LlmRequestBuilt {
        model: String,
        endpoint: String,
        token_estimate: usize,
        tools_count: usize,
        context_window: usize,
        /// Local item-text mix for the occupancy bar. Not meter/ring truth.
        #[serde(default)]
        token_breakdown: crate::session::estimate::ItemTokenBreakdown,
    },
    LlmCompleted {
        prompt_tokens: u64,
        completion_tokens: u64,
        cache_hit_tokens: u64,
        cache_miss_tokens: u64,
        stop_reason: String,
    },
    Compaction {
        kind: CompactionKind,
        detail: Option<String>,
    },
    PermissionResolved {
        tool: String,
        approved: bool,
        always: bool,
    },
    Error {
        code: ErrorCode,
        message: String,
    },
    SnapshotNotice {
        level: String,
        message: String,
    },
}

/// JSON-RPC 2.0 request envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequestEnvelope {
    pub jsonrpc: String,
    /// Request id — String, Number, or null (notification).
    pub id: serde_json::Value,
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcErrorBody>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcErrorBody {
    pub code: i64,
    pub message: String,
}

/// Transport-level operations that are not JSON-RPC methods.
///
/// Only `Quit` remains here: it is a connection-control operation with no
/// JSON-RPC method equivalent. All data-subscription operations
/// (`subscribe_logs`, `unsubscribe_logs`, `session/subscribe`,
/// `session/unsubscribe`) are handled as unified JSON-RPC methods.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportRequest {
    Quit,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffer_state_wire_is_seq_cursor_not_count_window() {
        let json = serde_json::to_value(BufferState {
            last_seq: 3,
            next_seq: 4,
            revision: 1,
        })
        .unwrap();
        assert_eq!(json["last_seq"], 3);
        assert_eq!(json["next_seq"], 4);
        assert!(json.get("len").is_none());
        assert!(json.get("committed_end").is_none());
    }

    #[test]
    fn buffer_load_result_includes_user_detail_before() {
        let json = serde_json::to_value(BufferLoadResult {
            session_id: "s1".into(),
            from_seq: 2,
            to_seq: 5,
            events: Vec::new(),
            subagent_bindings: HashMap::new(),
            user_detail_before: 3,
        })
        .unwrap();
        assert_eq!(json["user_detail_before"], 3);
        assert_eq!(json["from_seq"], 2);
        assert_eq!(json["to_seq"], 5);
    }
}

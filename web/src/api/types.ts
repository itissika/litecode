// --- ErrorCode (snake_case) ---

export type ErrorCode =
  | "llm_http"
  | "llm_parse"
  | "max_steps"
  | "compaction_failed"
  | "compaction_circuit_open"
  | "permission_denied"
  | "hook_blocked"
  | "tool_panic"
  | "tool_validation"
  | "cancelled"
  | "session_not_found"
  | "invalid_request"
  | "agent_already_running"
  | "invalid_revert_anchor"
  | "snapshot_unavailable"
  | "nothing_to_revert"
  | "nothing_to_compact"
  | "internal";

export interface StructuredError {
  code: ErrorCode;
  message: string;
}

export type CompactionKind =
  | "first_pass"
  | "second_pass"
  | "aggressive_truncate"
  | "blocked";

export type TurnPhase =
  | "idle"
  | "starting"
  | "compacting"
  | "calling_llm"
  | "streaming"
  | "executing_tools"
  | "cancelling"
  | "finalizing"
  | { awaiting_permission: { tool: string; rule_id: string; summary: string } }
  | { failed: { code: ErrorCode } };

// --- Responses Item (OpenAI / async-openai authority; minimal UI subset) ---
// Hand-written to match serde of async-openai response-types. Do not invent
// homemade transcript dialects parallel to Responses Items.

export type InputContent =
  | { type: "input_text"; text: string }
  | {
      type: "input_image";
      detail?: string;
      image_url?: string | null;
      file_id?: string | null;
    }
  | { type: "input_file"; [key: string]: unknown };

export type OutputContent =
  | {
      type: "output_text";
      text: string;
      annotations: unknown[];
      logprobs?: unknown;
    }
  | { type: "refusal"; refusal: string };

/** User / system / developer input message (`type: "message"`). */
export interface InputMessageItem {
  type: "message";
  role: "user" | "system" | "developer";
  content: InputContent[];
  status?: string;
  id?: string;
}

/** Assistant output message (`type: "message"`). */
export interface OutputMessageItem {
  type: "message";
  role: "assistant";
  id: string;
  status: string;
  content: OutputContent[];
  phase?: string;
}

export type MessageItem = InputMessageItem | OutputMessageItem;

export interface ReasoningItem {
  type: "reasoning";
  id?: string | null;
  summary: Array<{ type: "summary_text"; text: string } | { type: string; [k: string]: unknown }>;
  content?: Array<{ type: "reasoning_text"; text: string }>;
  encrypted_content?: string | null;
  status?: string;
}

export interface FunctionCallItem {
  type: "function_call";
  call_id: string;
  name: string;
  /** JSON string of arguments (OpenAI Responses shape). */
  arguments: string;
  id?: string;
  namespace?: string;
  status?: string;
}

export interface FunctionCallOutputItem {
  type: "function_call_output";
  call_id: string;
  /** String or structured content list (async-openai untagged enum). */
  output: string | InputContent[];
  id?: string | null;
  status?: string;
}

/**
 * Authority transcript atom. Known variants are typed; unknown `type` values
 * are accepted for forward-compat (passthrough, not rendered specially).
 * Catch-all must NOT use an index signature — that breaks discriminated narrowing
 * into MessageItem / FunctionCallItem (tsc treats content as unknown).
 */
export type Item =
  | MessageItem
  | ReasoningItem
  | FunctionCallItem
  | FunctionCallOutputItem
  | { type: string };

/**
 * Minimal ResponseStreamEvent subset used by the UI.
 * Handled semantic variants mutate Item-shaped state; lifecycle variants may no-op
 * (see `NON_SEMANTIC_STREAM_TYPES` in adapter). Anything else → shapeError.
 */
export type ResponseStreamEvent =
  | {
      type: "response.output_text.delta";
      sequence_number: number;
      item_id: string;
      output_index: number;
      content_index: number;
      delta: string;
      logprobs?: unknown;
    }
  | {
      type: "response.output_text.done";
      sequence_number: number;
      item_id: string;
      output_index: number;
      content_index: number;
      text: string;
      logprobs?: unknown;
    }
  | {
      type: "response.reasoning_text.delta";
      sequence_number?: number;
      item_id: string;
      output_index?: number;
      content_index?: number;
      delta: string;
    }
  | {
      type: "response.reasoning_text.done";
      sequence_number?: number;
      item_id: string;
      output_index?: number;
      content_index?: number;
      text: string;
    }
  | {
      type: "response.function_call_arguments.delta";
      sequence_number?: number;
      item_id: string;
      output_index?: number;
      delta: string;
    }
  | {
      type: "response.function_call_arguments.done";
      sequence_number?: number;
      item_id: string;
      output_index?: number;
      arguments: string;
      name?: string | null;
    }
  | {
      type: "response.output_item.added";
      sequence_number?: number;
      output_index?: number;
      item: Item;
    }
  | { type: string; [key: string]: unknown };

export interface SettingsSummary {
  revision: number;
  provider_endpoint: string | null;
  model_count: number;
  agent_count: number;
  catalog_count: number;
  log_level: string | null;
  effective_next_turn: boolean;
  restart_required: boolean;
  setup_guidance?: string | null;
}

export interface WireServerHello {
  version: string;
  /** Build channel: dev | nightly | official */
  version_channel?: string;
  session_id: string;
  project: string;
  /** Stable workspace identity from `.litecode/workspace.json`. */
  workspace_id: string;
  settings_revision: number;
  active_primary: string;
  primary_agents: PrimaryAgentInfo[];
  llm_ecosystem: string;
  models?: ModelInfo[];
}

/** Persisted SessionLog rows. `kind` is the only projection discriminator. */
export type ItemLogKind =
  | "item/user"
  | "item/assistant"
  | "item/tool_call"
  | "item/tool_result";

export interface ItemLogRow {
  seq: number;
  kind: ItemLogKind;
  body: Item;
  child_session_id?: string;
}

export interface CompactedLogRow {
  seq: number;
  kind: "compacted";
  body: { summary: string; from: number; to: number };
}

export interface HookPromptLogRow {
  seq: number;
  kind: "hook/prompt";
  body: { text: string; hook_run_id: string; placement?: string };
}

export interface JobExitReminderLogRow {
  seq: number;
  kind: "reminder/job_exit";
  body: { job_id?: string; reason: "exit" | "kill" | "timeout"; text: string };
}

export interface TurnAbortedReminderLogRow {
  seq: number;
  kind: "reminder/turn_aborted";
  body: { text: string };
}

export interface ControlLogRow {
  seq: number;
  kind: "turn/start" | "turn/end" | "request/header" | "request/context";
  body: Record<string, unknown>;
}

/** A committed wire row. Unknown kinds are skipped by HumanView at runtime. */
export type WireBufferEvent =
  | ItemLogRow
  | CompactedLogRow
  | HookPromptLogRow
  | JobExitReminderLogRow
  | TurnAbortedReminderLogRow
  | ControlLogRow;

/** HumanView row: a committed log row with transient UI-only state. */
export type HumanRow = WireBufferEvent & { streaming?: boolean };

export interface BufferLoaded {
  session_id: string;
  from_seq: number;
  to_seq: number;
  events: WireBufferEvent[];
  /** `subagent_launch` call_id → durable child session id (rebuild path). */
  subagent_bindings?: Record<string, string>;
  /** Server count of user-detail rows with seq < from_seq. */
  user_detail_before?: number;
}

export type BufferItemNotification = WireBufferEvent & {
  session_id: string;
  parent_session_id?: string;
};

/** Parent session: child created for a `subagent_launch` call (immediate bind). */
export interface SubagentBound {
  session_id: string;
  call_id: string;
  child_session_id: string;
}

export interface PrimaryAgentInfo {
  id: string;
  description: string;
}

export interface ModelInfo {
  id: string;
  api_model_id: string;
  label: string;
  context_window: number;
  adapter_id?: string;
}

export interface SettingsChanged {
  revision: number;
  summary: SettingsSummary;
}

export interface ServerStats {
  rss_kb: number | null;
  core_rss_kb?: number | null;
  embed_rss_kb?: number | null;
  lsp_rss_kb?: number | null;
  ts_ms: number;
}

export interface LogLine {
  ts_ms: number;
  level: string;
  target: string;
  message: string;
}

export interface SessionInfo {
  id: string;
  project: string;
  updated_at: number;
  preview: string;
  running: boolean;
  turn: TurnSnapshot | null;
  agent_id: string;
  model_id?: string | null;
  api_model_id: string;
  label?: string;
  parent_session_id?: string | null;
  parent_call_id?: string | null;
  /** Accumulated turn-step kinds for the current (or just-finished) turn.
   *  Appended on each `turn_step` event, cleared on `turn_started`. Purely a
   *  client-side flourish counter — never persisted, not sent back to the server. */
  step_kinds?: TurnStepKind[];
}

export interface SessionList {
  sessions: SessionInfo[];
}

export type SessionLifecycleEvent =
  | "deleted"
  | "turn_started"
  | "turn_updated"
  | "turn_finished"
  | "preview_updated"
  | "turn_step";

export type TurnStepKind = "reasoning" | "toolcall" | "text";

export interface SessionLifecycle {
  session_id: string;
  event: SessionLifecycleEvent;
  turn: TurnSnapshot | null;
  preview?: string;
  updated_at?: number;
  step_kind?: TurnStepKind;
}

export interface SessionAttached {
  session_id: string;
  turn: TurnSnapshot;
}

export interface WorkspaceChanged {
  paths: string[];
  kind: string;
}


export interface BufferState {
  last_seq: number;
  next_seq: number;
  revision: number;
}

export interface TurnSnapshot {
  turn_id: string;
  phase: TurnPhase;
  step: number;
  step_max: number;
  started_at_ms: number;
  awaiting_permission?: boolean;
}

export interface SessionSnapshot {
  session_id: string;
  project: string;
  agent_id: string;
  model_id?: string | null;
  api_model_id: string;
  label?: string;
  buffer: BufferState;
  turn: TurnSnapshot | null;
  context_window?: number;
  context_tokens_estimate?: number;
  compact_eligible?: boolean;
  compacting?: boolean;
  /** Last-known provider usage for context ring hydrate after refresh. */
  last_turn_token_stats?: TurnTokenStats | null;
  /** Session-total provider usage (Σ per-request; whole-session hit rate). */
  cumulative_token_stats?: TurnTokenStats | null;
  thinking_tier?: ThinkingTier;
  context_mode?: ContextMode;
  /**
   * Highest user-detail anchor whose file patch is nonempty.
   * Show "Revert files" when `userAnchorK <= max_file_revert_k`.
   * Absent / null = no file-level revert available.
   */
  max_file_revert_k?: number | null;
  /** Running agent bash jobs + wait_shell waiters (reconnect hydrate). */
  bash?: BashJobsSnapshot | null;
  /** Session-scoped todos (reconnect hydrate; not derived from the transcript). */
  todos?: {
    id: string;
    content: string;
    status: "pending" | "in_progress" | "completed";
    priority?: string | null;
  }[];
  /** Workspace-relative Markdown file for the session's active plan. */
  active_plan_path?: string | null;
}

export interface BashJob {
  id: string;
  call_id: string;
  command_preview: string;
  output_file: string;
  started_at_ms: number;
}

export interface BashWait {
  call_id: string;
  watching_id?: string | null;
  started_at_ms: number;
  deadline_ms?: number | null;
}

export interface BashJobsSnapshot {
  jobs: BashJob[];
  waits: BashWait[];
}

export interface BashJobsNotification extends BashJobsSnapshot {
  session_id: string;
}

export interface BashTailResult {
  text: string;
  truncated_on_disk: boolean;
  alive: boolean;
  exit_code: number | null;
}

export type ThinkingTier = "low" | "medium" | "high";
export type ContextMode = "standard" | "max";

export type TurnEndReason =
  | "completed"
  | "cancelled"
  | "max_steps"
  | "error"
  | "hook_blocked";

export interface TurnStarted {
  session_id: string;
  turn_id: string;
  input: string;
  step_max: number;
  /** Present on legacy forwarded subagent events; ignore for main transcript. */
  parent_session_id?: string;
}

export interface TurnEventEnvelope {
  session_id: string;
  turn_id: string;
  event: WireEvent;
  /** Present on legacy forwarded subagent events; ignore for main transcript. */
  parent_session_id?: string;
}

export interface TurnTokenStats {
  prompt_tokens: number;
  completion_tokens: number;
  cache_hit_tokens: number;
  cache_miss_tokens: number;
}

export interface TurnFinished {
  turn_id: string;
  reason: TurnEndReason;
  final_text: string | null;
  error: StructuredError | null;
  snapshot: SessionSnapshot;
  turn_token_stats?: TurnTokenStats;
  session_id: string;
  /** Present on legacy forwarded subagent events; ignore for main transcript. */
  parent_session_id?: string;
}

export type OperationKind =
  | "start"
  | "reload_session"
  | "new_session"
  | "delete_session"
  | "list_sessions"
  | "compact_session"
  | "revert_to_user_anchor"
  | "revert_files"
  | "set_active_primary"
  | "set_model"
  | "set_thinking_tier"
  | "set_context_mode";

export interface OperationResult {
  op: OperationKind;
  ok: boolean;
  error: StructuredError | null;
  snapshot: SessionSnapshot;
}

export interface CompactLifecycle {
  session_id: string;
  trigger: "manual" | "auto";
  stage: "started" | "succeeded" | "failed";
  operation_id?: string | null;
  error?: StructuredError | null;
  snapshot: SessionSnapshot;
}

/** @deprecated Use CompactLifecycle stage=started. Kept for one-release wire compat. */
export interface CompactStarted {
  session_id: string;
  operation_id: string;
  snapshot: SessionSnapshot;
  trigger?: "manual" | "auto";
}

export interface PermissionRequest {
  turn_id: string;
  request_id: string;
  tool: string;
  rule_id: string;
  summary: string;
  session_id: string;
  /** Present on legacy forwarded subagent events; ignore for main transcript. */
  parent_session_id?: string;
}

// --- WireEvent (Rust: #[serde(tag = "type", rename_all = "snake_case")]) ---

export type WireEvent =
  | { type: "stream_event"; event: ResponseStreamEvent }
  | {
      type: "todo_progress";
      pending: number;
      in_progress: number;
      completed: number;
      items: {
        id: string;
        content: string;
        status: "pending" | "in_progress" | "completed";
        priority?: string | null;
      }[];
    }
  | { type: "plan_changed"; active_plan_path: string | null }
  | { type: "phase_changed"; phase: TurnPhase; step: number }
  | { type: "step_started"; step: number; step_max: number }
  | {
      type: "llm_request_built";
      model: string;
      endpoint: string;
      token_estimate: number;
      tools_count: number;
      context_window?: number;
    }
  | {
      type: "llm_completed";
      prompt_tokens: number;
      completion_tokens: number;
      cache_hit_tokens: number;
      cache_miss_tokens: number;
      stop_reason: string;
    }
  | { type: "compaction"; kind: CompactionKind; detail: string | null }
  | { type: "hook_fired"; phase: string; action: string }
  | { type: "permission_resolved"; tool: string; approved: boolean; always: boolean }
  | { type: "error"; code: ErrorCode; message: string }
  | { type: "snapshot_notice"; level: string; message: string };

// --- JSON-RPC 2.0 types ---

export interface JsonRpcRequest {
  jsonrpc: "2.0";
  method: string;
  params?: Record<string, unknown>;
  id: string | number;
}

export interface JsonRpcResponse {
  jsonrpc: "2.0";
  result?: unknown;
  error?: { code: number; message: string; data?: unknown };
  id: string | number | null;
}

export interface JsonRpcNotification {
  jsonrpc: "2.0";
  method: string;
  params?: Record<string, unknown>;
}

export interface LspWireError {
  code: number;
  message: string;
}

export interface LspResult {
  id: number;
  result?: unknown;
  error?: LspWireError;
}

export interface LspNotification {
  method: string;
  params: Record<string, unknown>;
}

export type ConnectionState =
  | "disconnected"
  | "connecting"
  | "connected"
  | "reconnecting";

export type TransportRequest =
  | { subscribe_session: { session_id: string } }
  | { unsubscribe_session: { session_id: string } };

export type AgentRunState = "idle" | "running" | "cancelling";

export interface TurnMeta {
  phase: TurnPhase | null;
  step: number | null;
  stepMax: number | null;
  contextWindow: number;
  promptTokens: number;
  completionTokens: number;
  cacheHitTokens: number;
  cacheMissTokens: number;
  stopReason: string | null;
  lastCompaction: { kind: CompactionKind; detail: string | null } | null;
  lastHookFired: { phase: string; action: string } | null;
  lastPermissionResolved: { tool: string; approved: boolean; always: boolean } | null;
}

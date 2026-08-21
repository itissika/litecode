import type {
  FunctionCallItem,
  FunctionCallOutputItem,
  HumanRow,
  InputMessageItem,
  Item,
  MessageItem,
  OutputMessageItem,
  ReasoningItem,
  ResponseStreamEvent,
  TurnMeta,
  WireEvent,
} from "./types";
import { toWorkspacePath } from "../utils/path";

/** Extract an authority Item only from an `item/*` log body. */
export function itemFromRow(row: HumanRow): Item | undefined {
  switch (row.kind) {
    case "item/user":
    case "item/assistant":
    case "item/tool_call":
    case "item/tool_result":
      return row.body;
    default:
      return undefined;
  }
}

/** Compaction is a first-class log kind, rendered as a cut rather than summary text. */
export function isCompactCutRow(row: HumanRow): boolean {
  return row.kind === "compacted";
}

/** Only explicit user log rows are composer bubbles and revert anchors. */
export function isHumanUserRow(row: HumanRow): boolean {
  return row.kind === "item/user";
}

/** Injected and control-plane rows remain in the log but are hidden in HumanView. */
export function isHiddenHumanRow(row: HumanRow): boolean {
  return row.kind.startsWith("hook/") || row.kind.startsWith("reminder/") || row.kind.startsWith("turn/") || row.kind.startsWith("request/");
}

const HUMAN_VIEW_KINDS = new Set([
  "item/user",
  "item/assistant",
  "item/tool_call",
  "item/tool_result",
  "compacted",
]);

/** Kinds HumanView may group or render. Unknown/future kinds stay in the log. */
export function isHumanViewKind(kind: string): boolean {
  return HUMAN_VIEW_KINDS.has(kind);
}

/** True when the row has seq, kind, and body — the current buffer/item shape. */
export function isWellFormedBufferRow(ev: unknown): ev is HumanRow {
  if (ev === null || typeof ev !== "object") return false;
  const rec = ev as Record<string, unknown>;
  if (typeof rec.seq !== "number" || !Number.isFinite(rec.seq) || rec.seq < 0) {
    return false;
  }
  if (typeof rec.kind !== "string" || rec.kind.length === 0) return false;
  return "body" in rec;
}

/** Server prefix of user rows before this window; 0 when the window starts at seq 0. */
export function hydrateUserDetailBefore(
  fromSeq: number,
  serverValue: number | undefined,
  previous: number,
): number {
  if (fromSeq === 0) return 0;
  if (typeof serverValue === "number" && Number.isFinite(serverValue) && serverValue >= 0) {
    return serverValue;
  }
  return previous;
}

let nextPendingId = 0;
export function newPendingUserId(): string {
  nextPendingId += 1;
  return `pending-${nextPendingId}-${Date.now()}`;
}

/** Optimistic user text Item (OpenAI Responses shape). */
export function userTextItem(text: string): MessageItem {
  return {
    type: "message",
    role: "user",
    content: [{ type: "input_text", text }],
  };
}

export function emptyAssistantMessageItem(itemId: string): OutputMessageItem {
  return {
    type: "message",
    role: "assistant",
    id: itemId,
    status: "in_progress",
    content: [{ type: "output_text", text: "", annotations: [] }],
  };
}

export function emptyReasoningItem(itemId: string): ReasoningItem {
  return {
    type: "reasoning",
    id: itemId,
    summary: [],
    content: [{ type: "reasoning_text", text: "" }],
    status: "in_progress",
  };
}

export function emptyFunctionCallItem(
  itemId: string,
  opts?: { callId?: string; name?: string },
): FunctionCallItem {
  return {
    type: "function_call",
    id: itemId,
    call_id: opts?.callId ?? itemId,
    name: opts?.name ?? "",
    arguments: "",
    status: "in_progress",
  };
}

export function isMessageItem(item: Item): item is MessageItem {
  return item.type === "message" && "role" in item && "content" in item;
}

export function isUserMessage(item: Item): item is InputMessageItem {
  return isMessageItem(item) && item.role === "user";
}

/**
 * Absolute 0-based revert anchor for the explicit `item/user` row at `rowIndex`.
 * `userDetailBefore` is the server count of user rows before the loaded window.
 */
export function deriveUserAnchorK(
  messages: HumanRow[],
  rowIndex: number,
  userDetailBefore: number,
): number {
  let local = 0;
  const end = Math.max(0, Math.min(rowIndex, messages.length));
  for (let i = 0; i < end; i++) {
    if (isHumanUserRow(messages[i]!)) local += 1;
  }
  return userDetailBefore + local;
}

export function isAssistantMessage(item: Item): item is OutputMessageItem {
  return isMessageItem(item) && item.role === "assistant";
}

/**
 * Turn-level stream failure events. When one arrives there is no per-item
 * payload, so the caller (messageStore) invalidates every in-flight
 * `function_call` rather than leaving half-streamed calls stuck "in_progress".
 */
export function isStreamFailureEvent(event: ResponseStreamEvent): boolean {
  return event.type === "response.failed" || event.type === "error";
}

/** Mark any live (in_progress) FunctionCall Item as failed. */
export function markFunctionCallsFailed(items: Item[]): Item[] {
  return items.map((item) =>
    isFunctionCall(item) && item.status === "in_progress"
      ? { ...item, status: "failed" }
      : item,
  );
}

export function isInProgressItem(item: Item): boolean {
  return "status" in item && (item as { status?: string }).status === "in_progress";
}

export function isFunctionCall(item: Item): item is FunctionCallItem {
  return item.type === "function_call" && "call_id" in item && "name" in item;
}

export function isFunctionCallOutput(item: Item): item is FunctionCallOutputItem {
  return item.type === "function_call_output" && "call_id" in item && "output" in item;
}

export function isReasoningItem(item: Item): item is ReasoningItem {
  return item.type === "reasoning" && "summary" in item;
}

/** Authority id for matching live rows ↔ buffer/item (item.id, else call_id). */
export function itemAuthorityId(item: Item): string | undefined {
  if ("id" in item && typeof item.id === "string" && item.id.length > 0) {
    return item.id;
  }
  if (isFunctionCall(item) && item.call_id) return item.call_id;
  if (isFunctionCallOutput(item) && item.call_id) return item.call_id;
  return undefined;
}

/** React key is log seq. */
export function projectionRowKey(row: HumanRow): string {
  return String(row.seq);
}

/** Best-effort plain text from a message / reasoning Item. */
export function itemPlainText(item: Item): string {
  if (isAssistantMessage(item)) {
    return item.content
      .map((c) => (c.type === "output_text" ? c.text : c.type === "refusal" ? c.refusal : ""))
      .filter(Boolean)
      .join("\n");
  }
  if (isMessageItem(item)) {
    return item.content
      .map((c) => (c.type === "input_text" ? c.text : ""))
      .filter(Boolean)
      .join("\n");
  }
  if (isReasoningItem(item)) {
    const fromContent = (item.content ?? [])
      .map((c) => (c.type === "reasoning_text" ? c.text : ""))
      .filter(Boolean);
    if (fromContent.length > 0) return fromContent.join("\n");
    return item.summary
      .map((s) => ("text" in s && typeof s.text === "string" ? s.text : ""))
      .filter(Boolean)
      .join("\n");
  }
  if (isFunctionCall(item)) {
    return `${item.name}(${item.arguments})`;
  }
  if (isFunctionCallOutput(item)) {
    return functionCallOutputText(item);
  }
  return "";
}

/** True when a live Item shell has no visible/semantic content yet. */
export function isEmptyItemShell(item: Item): boolean {
  if (isAssistantMessage(item)) {
    return !itemPlainText(item);
  }
  if (isReasoningItem(item)) {
    return !itemPlainText(item);
  }
  if (isFunctionCall(item)) {
    return !item.name && !item.arguments;
  }
  return false;
}

export function functionCallOutputText(out: FunctionCallOutputItem): string {
  if (typeof out.output === "string") return out.output;
  return out.output
    .map((c) => (c.type === "input_text" ? c.text : `[${c.type}]`))
    .join("\n");
}

export function parseFunctionArguments(argumentsJson: string): unknown {
  try {
    return JSON.parse(argumentsJson);
  } catch {
    return argumentsJson;
  }
}

const FILE_TOOLS = new Set(["read", "write", "edit"]);

export function normalizeToolFilePath(
  filePath: string,
  projectRoot?: string | null,
): string | null {
  return toWorkspacePath(filePath, projectRoot);
}

export function extractToolFilePath(
  toolName: string,
  input: unknown,
  projectRoot?: string | null,
): string | null {
  if (!FILE_TOOLS.has(toolName)) return null;
  if (!input || typeof input !== "object") return null;

  const filePath = (input as Record<string, unknown>).file_path;
  if (typeof filePath !== "string" || !filePath) return null;

  return normalizeToolFilePath(filePath, projectRoot);
}

/**
 * Documented non-semantic / lifecycle ResponseStreamEvent types.
 * These may no-op in the FE projection. Text / tool / reasoning deltas must NOT be listed here.
 */
export const NON_SEMANTIC_STREAM_TYPES: ReadonlySet<string> = new Set([
  "response.created",
  "response.in_progress",
  "response.completed",
  "response.failed",
  "response.incomplete",
  // output_item.added is semantic for function_call (early name); handled below.
  "response.output_item.done",
  "response.content_part.added",
  "response.content_part.done",
  "response.file_search_call.in_progress",
  "response.file_search_call.searching",
  "response.file_search_call.completed",
  "response.web_search_call.in_progress",
  "response.web_search_call.searching",
  "response.web_search_call.completed",
  "response.reasoning_summary_part.added",
  "response.reasoning_summary_part.done",
  "response.image_generation_call.completed",
  "response.image_generation_call.generating",
  "response.image_generation_call.in_progress",
  "response.image_generation_call.partial_image",
  "response.mcp_call_arguments.delta",
  "response.mcp_call_arguments.done",
  "response.mcp_call.completed",
  "response.mcp_call.failed",
  "response.mcp_call.in_progress",
  "response.mcp_list_tools.completed",
  "response.mcp_list_tools.failed",
  "response.mcp_list_tools.in_progress",
  "response.code_interpreter_call.in_progress",
  "response.code_interpreter_call.interpreting",
  "response.code_interpreter_call.completed",
  "response.code_interpreter_call_code.delta",
  "response.code_interpreter_call_code.done",
  "error",
]);

export type ApplyStreamEventResult =
  | { kind: "upsert"; itemId: string; item: Item }
  | { kind: "noop" }
  | { kind: "error"; message: string };

function requireItemId(event: ResponseStreamEvent, _field = "item_id"): string | null {
  const raw = (event as { item_id?: unknown }).item_id;
  return typeof raw === "string" && raw.length > 0 ? raw : null;
}

function ensureAssistantMessage(existing: Item | undefined, itemId: string): OutputMessageItem | { error: string } {
  if (!existing) return emptyAssistantMessageItem(itemId);
  if (!isAssistantMessage(existing)) {
    return {
      error: `stream shape: expected assistant message Item for ${itemId}, got type=${String(existing.type)}`,
    };
  }
  return { ...existing, id: existing.id || itemId };
}

function ensureReasoning(existing: Item | undefined, itemId: string): ReasoningItem | { error: string } {
  if (!existing) return emptyReasoningItem(itemId);
  if (!isReasoningItem(existing)) {
    return {
      error: `stream shape: expected reasoning Item for ${itemId}, got type=${String(existing.type)}`,
    };
  }
  return { ...existing, id: existing.id ?? itemId };
}

function ensureFunctionCall(
  existing: Item | undefined,
  itemId: string,
  opts?: { name?: string; callId?: string },
): FunctionCallItem | { error: string } {
  if (!existing) {
    return emptyFunctionCallItem(itemId, { name: opts?.name, callId: opts?.callId });
  }
  if (!isFunctionCall(existing)) {
    return {
      error: `stream shape: expected function_call Item for ${itemId}, got type=${String(existing.type)}`,
    };
  }
  return {
    ...existing,
    id: existing.id ?? itemId,
    call_id:
      opts?.callId && opts.callId.length > 0 ? opts.callId : existing.call_id,
    name: opts?.name && opts.name.length > 0 ? opts.name : existing.name,
  };
}

function appendOutputText(item: OutputMessageItem, delta: string): OutputMessageItem {
  const content = [...item.content];
  const idx = content.findIndex((c) => c.type === "output_text");
  if (idx >= 0 && content[idx].type === "output_text") {
    content[idx] = { ...content[idx], text: content[idx].text + delta };
  } else {
    content.push({ type: "output_text", text: delta, annotations: [] });
  }
  return { ...item, content };
}

function setOutputText(item: OutputMessageItem, text: string): OutputMessageItem {
  const content = [...item.content];
  const idx = content.findIndex((c) => c.type === "output_text");
  if (idx >= 0 && content[idx].type === "output_text") {
    content[idx] = { ...content[idx], text };
  } else {
    content.push({ type: "output_text", text, annotations: [] });
  }
  return { ...item, content };
}

function appendReasoningText(item: ReasoningItem, delta: string): ReasoningItem {
  const content = [...(item.content ?? [])];
  const idx = content.findIndex((c) => c.type === "reasoning_text");
  if (idx >= 0) {
    content[idx] = { type: "reasoning_text", text: content[idx].text + delta };
  } else {
    content.push({ type: "reasoning_text", text: delta });
  }
  return { ...item, content };
}

function setReasoningText(item: ReasoningItem, text: string): ReasoningItem {
  const content = [...(item.content ?? [])];
  const idx = content.findIndex((c) => c.type === "reasoning_text");
  if (idx >= 0) {
    content[idx] = { type: "reasoning_text", text };
  } else {
    content.push({ type: "reasoning_text", text });
  }
  return { ...item, content };
}

/**
 * Apply a Responses stream event onto Item-shaped state.
 * Creates a typed shell on first delta for the event's `item_id`.
 * Does not write parallel partial strings.
 */
export function applyStreamEvent(
  existing: Item | undefined,
  event: ResponseStreamEvent,
): ApplyStreamEventResult {
  const type = event.type;

  if (NON_SEMANTIC_STREAM_TYPES.has(type)) {
    return { kind: "noop" };
  }

  if (type === "response.output_item.added") {
    const rawItem = (event as { item?: unknown }).item;
    if (!rawItem || typeof rawItem !== "object" || !("type" in rawItem)) {
      return {
        kind: "error",
        message: "stream shape: output_item.added missing item",
      };
    }
    const added = rawItem as {
      type: string;
      id?: unknown;
      call_id?: unknown;
      name?: unknown;
      arguments?: unknown;
      status?: unknown;
    };
    // Non-function_call output items remain lifecycle no-ops (messages, reasoning, …).
    if (added.type !== "function_call") {
      return { kind: "noop" };
    }
    const id =
      typeof added.id === "string" && added.id.length > 0 ? added.id : undefined;
    const callId =
      typeof added.call_id === "string" && added.call_id.length > 0
        ? added.call_id
        : undefined;
    const itemId = id ?? callId;
    if (!itemId) {
      return {
        kind: "error",
        message: "stream shape: output_item.added function_call missing id/call_id",
      };
    }
    const name = typeof added.name === "string" ? added.name : undefined;
    const base = ensureFunctionCall(existing, itemId, { name, callId });
    if ("error" in base) return { kind: "error", message: base.error };
    const next: FunctionCallItem = {
      ...base,
      name: name && name.length > 0 ? name : base.name,
      call_id: callId ?? base.call_id,
      status:
        typeof added.status === "string" ? added.status : (base.status ?? "in_progress"),
    };
    if (
      typeof added.arguments === "string" &&
      added.arguments.length > 0 &&
      !base.arguments
    ) {
      next.arguments = added.arguments;
    }
    return { kind: "upsert", itemId, item: next };
  }

  if (type === "response.output_text.delta") {
    const itemId = requireItemId(event);
    if (!itemId || typeof event.delta !== "string") {
      return { kind: "error", message: "stream shape: output_text.delta missing item_id/delta" };
    }
    const base = ensureAssistantMessage(existing, itemId);
    if ("error" in base) return { kind: "error", message: base.error };
    return { kind: "upsert", itemId, item: appendOutputText(base, event.delta) };
  }

  if (type === "response.output_text.done") {
    const itemId = requireItemId(event);
    if (!itemId || typeof event.text !== "string") {
      return { kind: "error", message: "stream shape: output_text.done missing item_id/text" };
    }
    const base = ensureAssistantMessage(existing, itemId);
    if ("error" in base) return { kind: "error", message: base.error };
    return { kind: "upsert", itemId, item: setOutputText(base, event.text) };
  }

  if (type === "response.reasoning_text.delta") {
    const itemId = requireItemId(event);
    if (!itemId || typeof event.delta !== "string") {
      return { kind: "error", message: "stream shape: reasoning_text.delta missing item_id/delta" };
    }
    const base = ensureReasoning(existing, itemId);
    if ("error" in base) return { kind: "error", message: base.error };
    return { kind: "upsert", itemId, item: appendReasoningText(base, event.delta) };
  }

  if (type === "response.reasoning_text.done") {
    const itemId = requireItemId(event);
    if (!itemId || typeof event.text !== "string") {
      return { kind: "error", message: "stream shape: reasoning_text.done missing item_id/text" };
    }
    const base = ensureReasoning(existing, itemId);
    if ("error" in base) return { kind: "error", message: base.error };
    return { kind: "upsert", itemId, item: setReasoningText(base, event.text) };
  }

  if (type === "response.function_call_arguments.delta") {
    const itemId = requireItemId(event);
    if (!itemId || typeof event.delta !== "string") {
      return {
        kind: "error",
        message: "stream shape: function_call_arguments.delta missing item_id/delta",
      };
    }
    const base = ensureFunctionCall(existing, itemId);
    if ("error" in base) return { kind: "error", message: base.error };
    return {
      kind: "upsert",
      itemId,
      item: { ...base, arguments: base.arguments + event.delta },
    };
  }

  if (type === "response.function_call_arguments.done") {
    const itemId = requireItemId(event);
    if (!itemId || typeof event.arguments !== "string") {
      return {
        kind: "error",
        message: "stream shape: function_call_arguments.done missing item_id/arguments",
      };
    }
    const name =
      "name" in event && typeof event.name === "string" ? event.name : undefined;
    const base = ensureFunctionCall(existing, itemId, { name });
    if ("error" in base) return { kind: "error", message: base.error };
    return {
      kind: "upsert",
      itemId,
      item: { ...base, arguments: event.arguments, name: name || base.name },
    };
  }

  // reasoning_summary_text is semantic (reasoning content) — project into Item.summary
  if (type === "response.reasoning_summary_text.delta") {
    const itemId = requireItemId(event);
    const delta = (event as { delta?: unknown }).delta;
    if (!itemId || typeof delta !== "string") {
      return {
        kind: "error",
        message: "stream shape: reasoning_summary_text.delta missing item_id/delta",
      };
    }
    const base = ensureReasoning(existing, itemId);
    if ("error" in base) return { kind: "error", message: base.error };
    const summary = [...base.summary];
    const last = summary[summary.length - 1];
    if (last && last.type === "summary_text" && "text" in last) {
      summary[summary.length - 1] = {
        type: "summary_text",
        text: String(last.text) + delta,
      };
    } else {
      summary.push({ type: "summary_text", text: delta });
    }
    return { kind: "upsert", itemId, item: { ...base, summary } };
  }

  if (type === "response.reasoning_summary_text.done") {
    const itemId = requireItemId(event);
    const text = (event as { text?: unknown }).text;
    if (!itemId || typeof text !== "string") {
      return {
        kind: "error",
        message: "stream shape: reasoning_summary_text.done missing item_id/text",
      };
    }
    const base = ensureReasoning(existing, itemId);
    if ("error" in base) return { kind: "error", message: base.error };
    return {
      kind: "upsert",
      itemId,
      item: { ...base, summary: [{ type: "summary_text", text }] },
    };
  }

  return {
    kind: "error",
    message: `stream shape: unhandled semantic event type \`${type}\` (cannot project to Item)`,
  };
}

/**
 * Detect unsafe seal mismatches when buffer/item stamps a live Item slot.
 * Same id space: live Item id/type must not contradict the committed authority Item.
 */
export function sealMismatchError(live: Item, committed: Item): string | null {
  const liveId = itemAuthorityId(live);
  const committedId = itemAuthorityId(committed);
  if (liveId && committedId && liveId !== committedId) {
    return `buffer/item seal mismatch: live id=${liveId} vs committed id=${committedId}`;
  }
  if (live.type !== committed.type) {
    return `buffer/item seal mismatch: live type=${live.type} vs committed type=${committed.type}`;
  }
  return null;
}

/** Visible payload only — status is stamped separately so a seal does not rebuild the tree. */
export function itemVisibleContentEqual(a: Item, b: Item): boolean {
  if (a.type !== b.type) return false;
  if (isUserMessage(a) && isUserMessage(b)) {
    return itemPlainText(a) === itemPlainText(b);
  }
  if (isAssistantMessage(a) && isAssistantMessage(b)) {
    return itemPlainText(a) === itemPlainText(b);
  }
  if (isFunctionCall(a) && isFunctionCall(b)) {
    return a.call_id === b.call_id && a.name === b.name && a.arguments === b.arguments;
  }
  if (isFunctionCallOutput(a) && isFunctionCallOutput(b)) {
    return a.call_id === b.call_id && JSON.stringify(a.output) === JSON.stringify(b.output);
  }
  if (isReasoningItem(a) && isReasoningItem(b)) {
    return itemPlainText(a) === itemPlainText(b);
  }
  return JSON.stringify(a) === JSON.stringify(b);
}

function stampTerminalFields(live: Item, committed: Item): Item {
  if (
    "status" in live &&
    "status" in committed &&
    live.status !== committed.status
  ) {
    return { ...live, status: committed.status } as Item;
  }
  return live;
}

export function mergeCommittedItem(live: Item, committed: Item): Item {
  const same = itemVisibleContentEqual(live, committed);
  return same ? stampTerminalFields(live, committed) : committed;
}

export function applyTurnEventMeta(event: WireEvent): Partial<TurnMeta> {
  switch (event.type) {
    case "phase_changed":
      return { phase: event.phase };
    case "step_started":
      return { step: event.step, stepMax: event.step_max };
    case "llm_request_built":
      return {
        // token_estimate is local budget telemetry — not ring truth.
        contextWindow: event.context_window ?? 0,
      };
    case "llm_completed":
      return {
        promptTokens: event.prompt_tokens,
        completionTokens: event.completion_tokens,
        cacheHitTokens: event.cache_hit_tokens,
        cacheMissTokens: event.cache_miss_tokens,
        stopReason: event.stop_reason,
      };
    case "compaction":
      return {
        lastCompaction: { kind: event.kind, detail: event.detail },
      };
    case "hook_fired":
      return {
        lastHookFired: { phase: event.phase, action: event.action },
      };
    case "permission_resolved":
      return {
        lastPermissionResolved: {
          tool: event.tool,
          approved: event.approved,
          always: event.always,
        },
      };
    default:
      return {};
  }
}

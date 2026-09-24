import type {
  FunctionCallItem,
  FunctionCallOutputItem,
  HumanRow,
  InputMessageItem,
  Item,
  JobExitReminderLogRow,
  MessageItem,
  OutputMessageItem,
  PlanExecuteLogRow,
  PlanReminderLogRow,
  ReasoningItem,
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

/** A durable background job event: visible as a system mark, never a chat bubble. */
export function isJobExitReminderRow(
  row: HumanRow,
): row is JobExitReminderLogRow {
  return row.kind === "reminder/job_exit";
}

/** A durable plan-review reminder: a system mark, never a chat bubble. */
export function isPlanReminderRow(row: HumanRow): row is PlanReminderLogRow {
  return row.kind === "reminder/plan";
}

/** The system-issued plan-execution trigger: a system mark, never a chat bubble. */
export function isPlanExecuteRow(row: HumanRow): row is PlanExecuteLogRow {
  return row.kind === "plan/execute";
}

/** HumanView kinds that are marks, not user/assistant bubbles. */
export type TranscriptMarkKind =
  | "compact_cut"
  | "job_exit"
  | "subagent_exit"
  | "plan"
  | "plan_execute";

export function isTranscriptMarkRow(row: HumanRow): boolean {
  return (
    isCompactCutRow(row) ||
    isJobExitReminderRow(row) ||
    isPlanReminderRow(row) ||
    isPlanExecuteRow(row)
  );
}

export function transcriptMarkKind(row: HumanRow): TranscriptMarkKind | null {
  if (isCompactCutRow(row)) return "compact_cut";
  if (isPlanExecuteRow(row)) return "plan_execute";
  if (isPlanReminderRow(row)) return "plan";
  if (isSubagentExitReminderRow(row)) return "subagent_exit";
  if (isJobExitReminderRow(row)) return "job_exit";
  return null;
}

/** Only explicit user log rows are composer bubbles and revert anchors. */
export function isHumanUserRow(row: HumanRow): boolean {
  return row.kind === "item/user";
}

/**
 * Text of a row that stands in for the optimistic composer bubble once it lands.
 * `item/user` is the normal case; `plan/execute` replaces the bubble with a mark
 * (its body is still the user `Item` the composer optimistically rendered).
 * `null` for any other row.
 */
export function optimisticUserSealText(row: HumanRow): string | null {
  if (isPlanExecuteRow(row))
    return isMessageItem(row.body) ? itemPlainText(row.body) : null;
  if (row.kind === "item/user")
    return isMessageItem(row.body) ? itemPlainText(row.body) : null;
  return null;
}

/** Injected and control-plane rows remain in the log but are hidden in HumanView. */
export function isHiddenHumanRow(row: HumanRow): boolean {
  return row.kind.startsWith("turn/") || row.kind.startsWith("request/");
}

export function isSubagentExitReminderRow(row: HumanRow): boolean {
  return (
    isJobExitReminderRow(row) &&
    isMessageItem(row.body) &&
    /^source: subagent$/m.test(itemPlainText(row.body))
  );
}

const HUMAN_VIEW_KINDS = new Set([
  "item/user",
  "item/assistant",
  "item/tool_call",
  "item/tool_result",
  "compacted",
  "reminder/job_exit",
  "reminder/plan",
  "plan/execute",
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
  // A row's lifecycle is the log's own field. Guessing "settled" when it is
  // missing is what silently swallowed live updates for reasoning rows.
  if (typeof rec.state !== "string" || rec.state.length === 0) return false;
  return "body" in rec;
}

/** Server prefix of user rows before this window; 0 when the window starts at seq 0. */
export function hydrateUserDetailBefore(
  fromSeq: number,
  serverValue: number | undefined,
  previous: number,
): number {
  if (fromSeq === 0) return 0;
  if (
    typeof serverValue === "number" &&
    Number.isFinite(serverValue) &&
    serverValue >= 0
  ) {
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

export function isFunctionCall(item: Item): item is FunctionCallItem {
  return item.type === "function_call" && "call_id" in item && "name" in item;
}

export function isFunctionCallOutput(
  item: Item,
): item is FunctionCallOutputItem {
  return (
    item.type === "function_call_output" &&
    "call_id" in item &&
    "output" in item
  );
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
      .map((c) =>
        c.type === "output_text"
          ? c.text
          : c.type === "refusal"
            ? c.refusal
            : "",
      )
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

/**
 * Latest non-empty assistant text in a transcript (subagent live-progress
 * summary). Skips reasoning items and streaming empty shells; returns the most
 * recent `output_text` block so a card header can show "what the subagent is
 * doing" in a single line.
 */
export function latestAssistantText(rows: HumanRow[]): string {
  let latest = "";
  for (const row of rows) {
    if (row.kind !== "item/assistant") continue;
    const item = itemFromRow(row);
    if (!item || !isAssistantMessage(item)) continue;
    const text = itemPlainText(item).trim();
    if (text) latest = text;
  }
  return latest;
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
    return (
      a.call_id === b.call_id &&
      a.name === b.name &&
      a.arguments === b.arguments
    );
  }
  if (isFunctionCallOutput(a) && isFunctionCallOutput(b)) {
    return (
      a.call_id === b.call_id &&
      JSON.stringify(a.output) === JSON.stringify(b.output)
    );
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
        tokenBreakdown: event.token_breakdown,
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

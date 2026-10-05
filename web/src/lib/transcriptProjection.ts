import {
  isFunctionCall,
  isFunctionCallOutput,
  isHiddenHumanRow,
  isHumanUserRow,
  isHumanViewKind,
  isMessageItem,
  isReasoningItem,
  isTranscriptMarkRow,
  itemFromRow,
  itemPlainText,
  projectionRowKey,
  transcriptMarkKind,
  userImageRefs,
} from "../api/adapter";
import type {
  BashExitReminderLogRow,
  FunctionCallItem,
  FunctionCallOutputItem,
  HumanRow,
  SubagentSettledLogRow,
} from "../api/types";
import { isToolCallLive } from "./toolCallLive";

/** Exit detail from a `reminder/bash_exit` body. Empty exits show the plain mark. */
export function jobExitDetail(
  body: BashExitReminderLogRow["body"] | undefined,
): string | undefined {
  const exits = body?.exits ?? [];
  if (exits.length === 0) return undefined;
  return exits
    .map((exit) =>
      exit.killed
        ? `${exit.job_id} · stopped by user (Kill)`
        : `${exit.job_id} · exit code ${exit.exit_code}`,
    )
    .join(", ");
}

export function subagentExitDetail(
  settled: SubagentSettledLogRow["body"]["settled"] | undefined,
): {
  detail: string;
  childId?: string;
} {
  const children = settled ?? [];
  const n = children.length || 1;
  const first = children[0];
  const who =
    n === 1
      ? first?.agent ||
        (first?.child_session_id
          ? first.child_session_id.slice(0, 8)
          : undefined)
      : undefined;
  const reason = n === 1 && first?.reason ? first.reason : undefined;
  const parts: string[] = n > 1 ? [`${n} settled`] : ["settled"];
  if (who) parts.push(who);
  if (reason) parts.push(reason);
  return { detail: parts.join(" · "), childId: first?.child_session_id };
}

export type RenderNode =
  | {
      kind: "text";
      text: string;
      key: string;
      streaming: boolean;
      live: boolean;
      incomplete?: boolean;
    }
  | {
      kind: "images";
      refs: string[];
      key: string;
      streaming: boolean;
      live: boolean;
    }
  | {
      kind: "reasoning";
      text: string;
      key: string;
      streaming: boolean;
      live: boolean;
      incomplete?: boolean;
    }
  | {
      kind: "compact_cut";
      summary?: string;
      key: string;
      streaming: boolean;
      live: false;
    }
  | {
      kind: "job_exit";
      detail?: string;
      key: string;
      streaming: boolean;
      live: false;
    }
  | {
      kind: "subagent_exit";
      detail?: string;
      childId?: string;
      key: string;
      streaming: boolean;
      live: false;
    }
  | { kind: "plan_execute"; key: string; streaming: boolean; live: false }
  | {
      kind: "tool";
      call: FunctionCallItem;
      output?: FunctionCallOutputItem;
      key: string;
      streaming: boolean;
      live: boolean;
    };

function outputsByCallId(
  rows: HumanRow[],
): Map<string, FunctionCallOutputItem> {
  const map = new Map<string, FunctionCallOutputItem>();
  for (const row of rows) {
    if (row.kind !== "item/tool_result") continue;
    const item = itemFromRow(row);
    if (item && isFunctionCallOutput(item)) map.set(item.call_id, item);
  }
  return map;
}

/**
 * Whether the log still holds this row in flight. The row's own `state` answers
 * it; a payload's `status` is provider content and may be absent entirely.
 */
function rowInProgress(row: HumanRow): boolean {
  return row.state === "in_progress";
}

/** Flatten HumanView rows into nodes; only `item/*` bodies are Items. */
export function rowsToNodes(rows: HumanRow[]): RenderNode[] {
  const outputs = outputsByCallId(rows);
  const outputInProgressByCallId = new Map<string, boolean>();
  for (const row of rows) {
    if (row.kind !== "item/tool_result") continue;
    const item = itemFromRow(row);
    if (item && isFunctionCallOutput(item)) {
      outputInProgressByCallId.set(item.call_id, rowInProgress(row));
    }
  }
  const nodes: RenderNode[] = [];
  for (const row of rows) {
    if (isHiddenHumanRow(row)) continue;
    const streaming = rowInProgress(row);
    const key = projectionRowKey(row);
    const mark = transcriptMarkKind(row);
    if (mark) {
      if (mark === "subagent_exit" && row.kind === "reminder/subagent_settled") {
        const parsed = subagentExitDetail(row.body.settled);
        nodes.push({
          kind: "subagent_exit",
          key,
          streaming: false,
          live: false,
          detail: parsed.detail,
          childId: parsed.childId,
        });
        continue;
      }
      const markDetail =
        mark === "job_exit" && row.kind === "reminder/bash_exit"
          ? jobExitDetail(row.body)
          : undefined;
      nodes.push({
        kind: mark,
        key,
        streaming: false,
        live: false,
        ...(mark === "compact_cut" && row.kind === "compacted"
          ? { summary: row.body.summary }
          : {}),
        ...(markDetail ? { detail: markDetail } : {}),
      });
      continue;
    }
    if (!isHumanViewKind(row.kind) || row.kind === "item/tool_result") continue;
    const item = itemFromRow(row);
    if (!item) continue;
    if (row.kind === "item/tool_call" && isFunctionCall(item)) {
      const output = outputs.get(item.call_id);
      const live = isToolCallLive({
        callStatus: item.status,
        hasOutput: output != null,
        outputInProgress: outputInProgressByCallId.get(item.call_id) === true,
      });
      nodes.push({
        kind: "tool",
        call: item,
        output,
        key,
        streaming,
        live,
      });
      continue;
    }
    if (row.kind === "item/assistant" && isReasoningItem(item)) {
      const text = itemPlainText(item);
      if (text) {
        nodes.push({
          kind: "reasoning",
          text,
          key,
          streaming,
          live: streaming,
          incomplete: item.status === "incomplete",
        });
      }
      continue;
    }
    if (
      (row.kind === "item/user" || row.kind === "item/assistant") &&
      isMessageItem(item)
    ) {
      const text = itemPlainText(item);
      if (row.kind === "item/user") {
        const refs = userImageRefs(item);
        if (refs.length > 0) {
          nodes.push({
            kind: "images",
            refs,
            key: `${key}:images`,
            streaming,
            live: streaming,
          });
        }
      }
      // Vendors emit whitespace-only content (e.g. "\n\n" before a tool call).
      // Such a message renders as nothing, but as an `output` node it would cut
      // the surrounding process group in half.
      if (text.trim()) {
        nodes.push({
          kind: "text",
          text,
          key,
          streaming,
          live: streaming,
          incomplete: item.status === "incomplete",
        });
      }
    }
  }
  return nodes;
}

export type NodeGroup = {
  type: "process" | "output" | "cut";
  nodes: RenderNode[];
};

export function processGroupHasTerminalStop(nodes: RenderNode[]): boolean {
  return nodes.some(
    (node) =>
      (node.kind === "reasoning" && node.incomplete === true) ||
      (node.kind === "tool" &&
        (node.call.status === "failed" || node.call.status === "incomplete")),
  );
}

export function groupNodes(nodes: RenderNode[]): NodeGroup[] {
  const groups: NodeGroup[] = [];
  let current: NodeGroup | null = null;

  for (const node of nodes) {
    if (
      node.kind === "compact_cut" ||
      node.kind === "job_exit" ||
      node.kind === "subagent_exit" ||
      node.kind === "plan_execute"
    ) {
      groups.push({ type: "cut", nodes: [node] });
      current = null;
      continue;
    }
    const isProcess = node.kind === "reasoning" || node.kind === "tool";
    const groupType = isProcess ? "process" : "output";
    if (!current || current.type !== groupType) {
      current = { type: groupType, nodes: [] };
      groups.push(current);
    }
    current.nodes.push(node);
  }
  return groups;
}

/**
 * Group consecutive rows for display: each user message is its own bubble;
 * consecutive non-user Items (live shells or sealed) coalesce into one assistant bubble
 * so process/output grouping still works across Item atoms.
 *
 * Transcript marks (`compacted`, visible reminders) are their own barrier
 * (not pushed into the previous assistant
 * bubble, not glued onto the next user bubble).
 */
export function groupRowsForBubbles(rows: HumanRow[]): HumanRow[][] {
  const groups: HumanRow[][] = [];
  let current: HumanRow[] = [];

  const flush = () => {
    if (current.length) {
      groups.push(current);
      current = [];
    }
  };

  for (const row of rows) {
    if (!isHumanViewKind(row.kind) || isHiddenHumanRow(row)) continue;
    if (isTranscriptMarkRow(row)) {
      flush();
      groups.push([row]);
      continue;
    }
    if (isHumanUserRow(row)) {
      flush();
      groups.push([row]);
    } else {
      current.push(row);
    }
  }
  flush();
  return groups;
}

/** Stable virtual-item identity for a bubble: min(seq) in the group. */
export function bubbleIdentity(bubbles: HumanRow[][], index: number): string {
  return bubbleKey(bubbles[index] ?? [], index);
}

function bubbleKey(group: HumanRow[], index: number): string {
  let min: number | undefined;
  for (const row of group) {
    if (min === undefined || row.seq < min) min = row.seq;
  }
  return min === undefined ? String(index) : String(min);
}

/**
 * True when this user row's turn-start stem (`seq + 1`) is at or before the
 * latest stem that recorded a revertible file patch.
 */
export function canRevertFiles(
  userSeq: number,
  maxFileRevertSeq: number | null | undefined,
): boolean {
  return maxFileRevertSeq != null && userSeq + 1 <= maxFileRevertSeq;
}

/** User message or transcript mark: a row that starts a new bubble by itself. */
export function isBarrierRow(row: HumanRow): boolean {
  return isHumanUserRow(row) || isTranscriptMarkRow(row);
}

/**
 * Drop the partial assistant run at the start of a history window.
 *
 * Paging cuts on a seq boundary, which almost always lands inside an assistant
 * turn. Rendering that fragment gives it `min(seq)` as its bubble key; the next
 * page then merges the real start of the turn in and the key changes, so the
 * virtualizer loses its scroll anchor and FoldCards lose their open state.
 * The first rendered bubble of a partial window is therefore always a barrier
 * row, whose key cannot change when older rows arrive. `fromSeq === 0` is the
 * real start of the log, so nothing is dropped. A window with no barrier at
 * all renders nothing until paging finds one.
 */
export function trimPartialHead(rows: HumanRow[], fromSeq: number): HumanRow[] {
  if (fromSeq <= 0) return rows;
  const start = rows.findIndex(isBarrierRow);
  if (start < 0) return [];
  if (start === 0) return rows;
  return rows.slice(start);
}

export interface Bubble {
  key: string;
  rows: HumanRow[];
  /** First non-mark row, if this bubble has one. */
  first?: HumanRow;
  isUser: boolean;
  markOnly: boolean;
  followedByUser: boolean;
  /** Next bubble is a system mark (compact, background exit, …), which ends the segment. */
  followedByMark: boolean;
}

/** One pass over the visible rows: bubble identity and whether the next bubble is a user message. */
export function projectBubbles(rows: HumanRow[]): Bubble[] {
  const groups = groupRowsForBubbles(rows);
  const bubbles: Bubble[] = groups.map((group, index) => {
    const first = group.find((row) => !isTranscriptMarkRow(row));
    const isUser = first != null && isHumanUserRow(first);
    const bubble: Bubble = {
      key: bubbleKey(group, index),
      rows: group,
      first,
      isUser,
      markOnly: group.every(isTranscriptMarkRow),
      followedByUser: false,
      followedByMark: false,
    };
    return bubble;
  });
  for (let i = 0; i < bubbles.length - 1; i++) {
    const next = bubbles[i + 1]!;
    bubbles[i]!.followedByUser = next.isUser;
    bubbles[i]!.followedByMark = next.markOnly;
  }
  return bubbles;
}

/** Plain text of a bubble's first message row, for height estimation. */
export function bubblePlainText(bubble: Bubble): string {
  const row = bubble.first;
  if (!row) return "";
  const item = itemFromRow(row);
  if (!item || !isMessageItem(item)) return "";
  return itemPlainText(item);
}

/** Image parts on a user bubble's first row. */
export function bubbleImageCount(bubble: Bubble): number {
  const row = bubble.first;
  if (!row) return 0;
  const item = itemFromRow(row);
  return item ? userImageRefs(item).length : 0;
}

/** `py-4` on a user bubble. */
const USER_BUBBLE_PAD = 32;
/** `py-2` on an assistant bubble. */
const ASSISTANT_BUBBLE_PAD = 16;
/** `0.89rem * 1.75` prose line-height, rounded. */
const USER_LINE_HEIGHT = 25;
/** Reading measure is 72ch; a long line wraps there. */
const USER_CHARS_PER_LINE = 72;
/** FoldCard header, or one collapsed tool row inside an open process group. */
const PROCESS_ROW = 28;

/** One image strip under the text padding. */
const USER_IMAGE_STRIP = 128;

function wrappedLineCount(text: string): number {
  let lines = 0;
  for (const part of text.split("\n")) {
    lines += Math.max(1, Math.ceil(part.length / USER_CHARS_PER_LINE));
  }
  return lines;
}

/**
 * Rough height of a user bubble before it is measured. The virtualizer
 * replaces it with the real height on first layout.
 */
export function estimateUserBubbleHeight(
  text: string,
  imageCount = 0,
): number {
  const lines =
    text.length > 0 ? wrappedLineCount(text) : imageCount === 0 ? 1 : 0;
  return (
    USER_BUBBLE_PAD +
    lines * USER_LINE_HEIGHT +
    (imageCount > 0 ? USER_IMAGE_STRIP : 0)
  );
}

/**
 * Rough height of an assistant bubble from the rows already in hand.
 * A closed process group is only its header. An open one adds reasoning
 * lines and one row per tool. Prose uses the same line measure as user text.
 * The virtualizer replaces this with the measured height.
 */
export function estimateAssistantBubbleHeight(bubble: Bubble): number {
  const groups = groupNodes(rowsToNodes(bubble.rows));
  let height = ASSISTANT_BUBBLE_PAD;
  if (groups.length === 0) return height + USER_LINE_HEIGHT;
  for (let index = 0; index < groups.length; index++) {
    const group = groups[index]!;
    if (group.type === "cut") {
      height += PROCESS_ROW;
      continue;
    }
    if (group.type === "output") {
      for (const node of group.nodes) {
        if (node.kind === "text") height += wrappedLineCount(node.text) * USER_LINE_HEIGHT;
        else if (node.kind === "images") height += USER_IMAGE_STRIP;
      }
      continue;
    }
    height += PROCESS_ROW;
    const followedByMessage =
      groups[index + 1]?.type === "output" ||
      bubble.followedByUser ||
      bubble.followedByMark;
    const open =
      !followedByMessage && !processGroupHasTerminalStop(group.nodes);
    if (!open) continue;
    for (const node of group.nodes) {
      if (node.kind === "reasoning") {
        height += wrappedLineCount(node.text) * USER_LINE_HEIGHT;
      } else if (node.kind === "tool") {
        height += PROCESS_ROW;
      }
    }
  }
  return height;
}

/** Find the virtual bubble that contains a transcript seq. */
export function locateSeq(bubbles: Bubble[], seq: number): number | null {
  for (let i = 0; i < bubbles.length; i++) {
    if (bubbles[i]!.rows.some((row) => row.seq === seq)) return i;
  }
  return null;
}

/** FoldCard id shared by the bubble chrome and bash-job reveal. */
export function foldId(
  sessionId: string,
  bubbleKey: string,
  part: string,
): string {
  return `${sessionId}:${bubbleKey}:${part}`;
}

/**
 * Find the virtual bubble + FoldCard ids for a live bash job's tool card.
 * Scans rows for the call first so only the matching bubble is projected.
 */
export function locateBashTool(
  bubbles: Bubble[],
  callId: string,
  sessionId: string,
): { bubbleIndex: number; foldIds: string[] } | null {
  for (let i = 0; i < bubbles.length; i++) {
    const bubble = bubbles[i]!;
    let found = false;
    for (const row of bubble.rows) {
      if (row.kind !== "item/tool_call") continue;
      const item = itemFromRow(row);
      if (item && isFunctionCall(item) && item.call_id === callId) {
        found = true;
        break;
      }
    }
    if (!found) continue;
    const groups = groupNodes(rowsToNodes(bubble.rows));
    for (let gi = 0; gi < groups.length; gi++) {
      const grouped = groups[gi]!;
      for (const node of grouped.nodes) {
        if (node.kind !== "tool" || node.call.call_id !== callId) continue;
        const foldIds = [foldId(sessionId, bubble.key, `tool:${callId}`)];
        if (grouped.type === "process") {
          foldIds.unshift(foldId(sessionId, bubble.key, `process:${gi}`));
        }
        return { bubbleIndex: i, foldIds };
      }
    }
  }
  return null;
}

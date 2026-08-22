import { BrainIcon, PencilIcon, TerminalIcon, WrenchIcon } from "@phosphor-icons/react";
import { memo, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type CSSProperties, type RefObject } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";

import {
  deriveUserAnchorK,
  isCompactCutRow,
  isFunctionCall,
  isFunctionCallOutput,
  isHumanUserRow,
  isHumanViewKind,
  isInProgressItem,
  isJobExitReminderRow,
  isMessageItem,
  isReasoningItem,
  itemFromRow,
  itemPlainText,
  projectionRowKey,
} from "../api/adapter";
import type {
  FunctionCallItem,
  FunctionCallOutputItem,
  HumanRow,
} from "../api/types";
import { AgentMarkdown } from "./AgentMarkdown";
import { CategoryCount } from "./CategoryCount";
import { FoldCard } from "./FoldCard";
import { InlineToolRow } from "./InlineToolRow";
import { requestFoldCardOpen } from "./foldCardState";
import { useSessionStore } from "../stores/sessionStore";
import { useEditorStore } from "../stores/editorStore";
import { useTurnStore } from "../stores/turnStore";
import { WaveText } from "./WaveText";
import { isInlineTool, processToolBucket } from "../lib/toolCategory";
import { useStickToBottom } from "../lib/scrollStick";
import { ToolCallCard } from "./ToolCallCard";
import { isToolCallLive, processGroupAutoOpen } from "./toolCallStatus";
import { MiniChatInput, type MiniChatInputSettings } from "./MiniChatInput";

type RenderNode =
  | { kind: "text"; text: string; key: string; streaming: boolean; live: boolean; incomplete?: boolean }
  | { kind: "reasoning"; text: string; key: string; streaming: boolean; live: boolean; incomplete?: boolean }
  | { kind: "compact_cut"; key: string; streaming: boolean }
  | { kind: "job_exit"; reason: "exit" | "kill" | "timeout"; key: string; streaming: boolean; live: false }
  | {
      kind: "tool";
      call: FunctionCallItem;
      output?: FunctionCallOutputItem;
      key: string;
      streaming: boolean;
      live: boolean;
    };

export interface EditingUserAnchor {
  bubbleKey: string;
  userAnchorK: number;
  draft: string;
  settings: MiniChatInputSettings;
}

/** Cut mark between transcript items — not a divider bubble, not summary text. */
export function CompactCutMark() {
  return (
    <div
      role="separator"
      aria-label="Context compacted here"
      className="flex items-center gap-1.5 py-1"
    >
      <span className="h-1 w-1 shrink-0 rounded-full bg-(--_dk-text-disabled)" />
      <span className="text-dk-2xs text-(--_dk-text-disabled)">compaction point</span>
    </div>
  );
}

export function JobExitMark({ reason }: { reason: "exit" | "kill" | "timeout" }) {
  const label = reason === "kill"
    ? "Background terminal stopped"
    : reason === "timeout"
      ? "Background terminal timed out"
      : "Background terminal exited";
  return (
    <div
      role="status"
      aria-label={label}
      className="flex items-center gap-1.5 py-1"
    >
      <span className="h-1 w-1 shrink-0 rounded-full bg-(--_dk-text-disabled)" />
      <span className="text-dk-2xs text-(--_dk-text-disabled)">{label}</span>
    </div>
  );
}

/** Transient "compacting in progress" line — shown while a compaction runs,
 *  replaced by the `CompactCutMark` (compaction point) when the checkpoint
 *  item lands. Uses the same per-character wave as the wait-shell text. */
export function CompactingMark() {
  return (
    <div
      role="status"
      aria-label="Compacting context"
      data-testid="compacting-now"
      className="flex items-center gap-1.5 py-1"
    >
      <span className="h-1 w-1 shrink-0 rounded-full bg-(--_dk-text-disabled)" />
      <WaveText text="compacting…" className="text-dk-2xs" />
    </div>
  );
}

/** Bash-exit auto-turn input is a user-role reminder; show a one-line notice. */
export function SystemReminderMark() {
  return (
    <div
      role="status"
      aria-label="Terminal killed"
      className="flex items-center gap-1.5 py-1"
    >
      <span className="h-1 w-1 shrink-0 rounded-full bg-(--_dk-text-disabled)" />
      <span className="text-dk-2xs text-(--_dk-text-disabled)">Terminal killed</span>
    </div>
  );
}

function outputsByCallId(rows: HumanRow[]): Map<string, FunctionCallOutputItem> {
  const map = new Map<string, FunctionCallOutputItem>();
  for (const row of rows) {
    if (row.kind !== "item/tool_result") continue;
    const item = itemFromRow(row);
    if (item && isFunctionCallOutput(item)) map.set(item.call_id, item);
  }
  return map;
}

function rowInProgress(row: HumanRow): boolean {
  const item = itemFromRow(row);
  return row.streaming === true || (item ? isInProgressItem(item) : false);
}

/** Flatten HumanView rows into nodes; only `item/*` bodies are Items. */
export function rowsToNodes(rows: HumanRow[]): RenderNode[] {
  const outputs = outputsByCallId(rows);
  const outputInProgressByCallId = new Map<string, boolean>();
  for (const row of rows) {
    if (row.kind !== "item/tool_result") continue;
    const item = itemFromRow(row);
    if (item && isFunctionCallOutput(item)) {
      outputInProgressByCallId.set(item.call_id, isInProgressItem(item));
    }
  }
  const nodes: RenderNode[] = [];
  for (const row of rows) {
    const streaming = rowInProgress(row);
    const key = projectionRowKey(row);
    if (isCompactCutRow(row)) {
      nodes.push({ kind: "compact_cut", key, streaming: false });
      continue;
    }
    if (isJobExitReminderRow(row)) {
      nodes.push({ kind: "job_exit", reason: row.body.reason, key, streaming: false, live: false });
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
        kind: "tool", call: item, output, key, streaming, live,
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
          live: isInProgressItem(item),
          incomplete: item.status === "incomplete",
        });
      }
      continue;
    }
    if ((row.kind === "item/user" || row.kind === "item/assistant") && isMessageItem(item)) {
      const text = itemPlainText(item);
      if (text) {
        nodes.push({
          kind: "text",
          text,
          key,
          streaming,
          live: isInProgressItem(item),
          incomplete: item.status === "incomplete",
        });
      }
    }
  }
  return nodes;
}

type NodeGroup = { type: "process" | "output" | "cut"; nodes: RenderNode[] };

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
    if (node.kind === "compact_cut" || node.kind === "job_exit") {
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

export function NodeView({
  node,
  streaming = false,
  projectRoot,
  onOpenFile,
  sessionId,
  bubbleKey,
}: {
  node: RenderNode;
  streaming?: boolean;
  projectRoot?: string | null;
  onOpenFile?: (path: string) => void;
  sessionId?: string;
  /** Stable bubble identity (projection key of the bubble's first row), used to
   *  namespace this node's FoldCard state across virtual-list remounts. */
  bubbleKey?: string;
}) {
  switch (node.kind) {
    case "reasoning":
      return (
        <FoldCard
          id={
            bubbleKey && sessionId
              ? `${sessionId}:${bubbleKey}:reasoning:${node.key}`
              : undefined
          }
          className="text-sm"
          contentClassName="text-(--_dk-text-secondary)"
          icon={<BrainIcon size={13} aria-hidden className="shrink-0 text-(--_dk-text-muted)" />}
          label={node.incomplete ? "Reasoning (incomplete)" : "Reasoning"}
          autoOpen={node.live}
          streaming={streaming}
        >
          <AgentMarkdown text={node.text} streaming={streaming} />
        </FoldCard>
      );
    case "text":
      return (
        <div className="text-dk-base text-(--_dk-text-primary) pl-(--_dk-indent-card-head)">
          <AgentMarkdown text={node.text} streaming={streaming} />
          {node.incomplete && !streaming ? (
            <div className="mt-1 text-dk-2xs italic text-(--_dk-text-disabled)">
              Output incomplete
            </div>
          ) : null}
        </div>
      );
    case "tool":
      if (isInlineTool(node.call.name)) {
        return (
          <InlineToolRow
            call={node.call}
            output={node.output}
            streaming={node.live}
            sessionId={sessionId}
          />
        );
      }
      return (
        <ToolCallCard
          call={node.call}
          output={node.output}
          streaming={node.live}
          projectRoot={projectRoot ?? null}
          onOpenFile={(path) => onOpenFile?.(path)}
          sessionId={sessionId}
          foldCardId={
            bubbleKey && sessionId
              ? `${sessionId}:${bubbleKey}:tool:${node.call.call_id}`
              : undefined
          }
        />
      );
    case "compact_cut":
      return <CompactCutMark />;
    case "job_exit":
      return <JobExitMark reason={node.reason} />;
  }
}

export function ProcessGroup({
  nodes,
  streaming,
  autoOpen,
  sessionId,
  bubbleKey,
  groupIndex,
}: {
  nodes: RenderNode[];
  streaming: boolean;
  autoOpen: boolean;
  sessionId?: string;
  /** Stable bubble identity, used to namespace this group's FoldCard state. */
  bubbleKey?: string;
  /** Index of this process group within its bubble (for a unique FoldCard id). */
  groupIndex: number;
}) {
  const project = useSessionStore((s) => s.project);
  const openFile = useEditorStore((s) => s.openFile);

  const reasoningCount = nodes.filter((n) => n.kind === "reasoning").length;
  let bashCount = 0;
  let editCount = 0;
  let toolCount = 0;
  for (const node of nodes) {
    if (node.kind !== "tool") continue;
    const bucket = processToolBucket(node.call.name);
    if (bucket === "bash") bashCount += 1;
    else if (bucket === "edit") editCount += 1;
    else if (bucket === "tool") toolCount += 1;
  }

  const ariaParts: string[] = [];
  if (reasoningCount > 0) {
    ariaParts.push(`${reasoningCount} reasoning`);
  }
  if (bashCount > 0) {
    ariaParts.push(`${bashCount} bash`);
  }
  if (editCount > 0) {
    ariaParts.push(`${editCount} edit`);
  }
  if (toolCount > 0) {
    ariaParts.push(`${toolCount} tool${toolCount !== 1 ? "s" : ""}`);
  }
  const headerAriaLabel = ariaParts.join(", ") || "Process";

  return (
    <FoldCard
      id={
        bubbleKey && sessionId
          ? `${sessionId}:${bubbleKey}:process:${groupIndex}`
          : undefined
      }
      icon={null}
      headerClassName="text-dk-sm text-(--_dk-text-secondary)"
      label={
        <span className="flex min-w-0 flex-1 items-center gap-2.5">
          <CategoryCount
            icon={
              <BrainIcon
                size={14}
                aria-hidden
                className="shrink-0 text-(--_dk-text-secondary)"
              />
            }
            count={reasoningCount}
            noun="reasoning"
          />
          <CategoryCount
            icon={
              <TerminalIcon
                size={14}
                aria-hidden
                className="shrink-0 text-(--_dk-text-secondary)"
              />
            }
            count={bashCount}
            noun="bash"
          />
          <CategoryCount
            icon={
              <PencilIcon
                size={14}
                aria-hidden
                className="shrink-0 text-(--_dk-text-secondary)"
              />
            }
            count={editCount}
            noun="edit"
          />
          <CategoryCount
            icon={
              <WrenchIcon
                size={14}
                aria-hidden
                className="shrink-0 text-(--_dk-amber-500)"
              />
            }
            count={toolCount}
            noun="tool"
          />
        </span>
      }
      headerAriaLabel={headerAriaLabel}
      autoOpen={autoOpen}
      streaming={streaming}
    >
      <div className="space-y-1">
        {nodes.map((node) => (
          <NodeView
            key={node.key}
            node={node}
            streaming={node.streaming}
            projectRoot={project}
            onOpenFile={(path) => void openFile(path)}
            sessionId={sessionId}
            bubbleKey={bubbleKey}
          />
        ))}
      </div>
    </FoldCard>
  );
}

/** Bubble for a contiguous run of rows that share the same speaker side. */
function ItemBubbleImpl({
  rows,
  userAnchorK,
  showRevert,
  sessionId,
  bubbleKey,
  editingAnchor,
  onEditAnchor,
  onDismissEdit,
  miniPhase,
  onMiniAnimationEnd,
}: {
  rows: HumanRow[];
  userAnchorK?: number;
  showRevert: boolean;
  isRunning: boolean;
  sessionId: string;
  /** Stable identity of this bubble (`min(seq)`). Namespaces
   *  child FoldCard open-state so it survives virtual-list remounts. */
  bubbleKey?: string;
  showRevertFiles?: boolean;
  editingAnchor: EditingUserAnchor | null;
  onEditAnchor: (anchor: EditingUserAnchor) => void;
  onDismissEdit: () => void;
  miniPhase: "idle" | "entering" | "visible" | "exiting";
  onMiniAnimationEnd: () => void;
}) {
  const sessionSettings = useSessionStore((s) => s.byId.get(sessionId));
  const replayFromAnchor = useTurnStore((s) => s.replayFromAnchor);
  const replaying = useTurnStore((s) => s.byId.get(sessionId)?.replaying ?? false);
  const first = rows.find((r) => !isCompactCutRow(r)) ?? rows[0];
  const isUser = first != null && isHumanUserRow(first);
  const nodes = rowsToNodes(rows);
  const streaming =
    rows.some((r) => rowInProgress(r)) || nodes.some((n) => n.streaming);
  const hasContent = nodes.length > 0 || !streaming;
  const groups = groupNodes(nodes);
  const userText = nodes.find((node) => node.kind === "text")?.text ?? "";
  const editing =
    isUser &&
    bubbleKey !== undefined &&
    editingAnchor?.bubbleKey === bubbleKey &&
    userAnchorK !== undefined;

  const body = !hasContent ? (
    <span className="inline-block h-4 w-2 bg-(--_dk-text-muted)" />
  ) : (
    groups.map((group, gi) => {
      if (group.type === "cut") {
        return group.nodes.map((n) => (
          <NodeView key={n.key} node={n} sessionId={sessionId} bubbleKey={bubbleKey} />
        ));
      }
      if (group.type === "process") {
        const groupLive = group.nodes.some((n) => n.kind !== "compact_cut" && n.live);
        const followedByMessage = groups[gi + 1]?.type === "output";
        const hasTerminalStop = processGroupHasTerminalStop(group.nodes);
        const groupAutoOpen = processGroupAutoOpen({
          followedByMessage,
          hasTerminalStop,
        });
        return (
          <ProcessGroup
            // Index within this bubble — stable as the group grows and across
            // live→seal (must NOT use row.id / first-node key; those remount).
            key={`process-${gi}`}
            nodes={group.nodes}
            streaming={groupLive}
            autoOpen={groupAutoOpen}
            sessionId={sessionId}
            bubbleKey={bubbleKey}
            groupIndex={gi}
          />
        );
      }
      return group.nodes.map((n) => (
        <NodeView
          key={n.key}
          node={n}
          streaming={n.streaming}
          sessionId={sessionId}
          bubbleKey={bubbleKey}
        />
      ));
    })
  );

  return (
    <div className={isUser ? "py-4" : "py-2"}>
      {isUser ? (
        editing ? (
          <div
            className={`relative z-10 w-full origin-bottom ${
              miniPhase === "entering" || miniPhase === "exiting"
                ? "overflow-hidden"
                : "overflow-visible"
            } ${
              miniPhase === "entering" ? "animate-mini-chat-enter" : ""
            } ${miniPhase === "exiting" ? "animate-mini-chat-exit" : ""}`}
            onAnimationEnd={onMiniAnimationEnd}
          >
            <MiniChatInput
              sessionId={sessionId}
              draft={editingAnchor.draft}
              settings={editingAnchor.settings}
              disabled={replaying}
              onDismiss={onDismissEdit}
              onChange={(draft, settings) => {
                onEditAnchor({ ...editingAnchor, draft, settings });
              }}
              onSubmit={(input, settings) => {
                onDismissEdit();
                void replayFromAnchor(sessionId, userAnchorK, input, settings);
              }}
            />
          </div>
        ) : (
        <div
          data-user-message-bubble
          className="flex cursor-text items-start gap-2"
          onClick={() => {
            if (!showRevert || userAnchorK === undefined || !bubbleKey || editing) return;
            onEditAnchor({
              bubbleKey,
              userAnchorK,
              draft: userText,
              settings: {
                primaryId: sessionSettings?.activePrimary ?? "default",
                modelId: sessionSettings?.modelId ?? "",
                thinkingTier: sessionSettings?.thinkingTier ?? "medium",
                contextMode: sessionSettings?.contextMode ?? "standard",
              },
            });
          }}
        >
          <span className="mt-[7px] h-1.5 w-1.5 shrink-0 rounded-full bg-(--_dk-accent-hover)" />
          <div className="min-w-0 flex-1">{body}</div>
        </div>
        )
      ) : (
        <div className="flex items-start gap-2">
          <span className="mt-[7px] h-1.5 w-1.5 shrink-0 rounded-full bg-(--_dk-text-muted)" />
          <div className="min-w-0 flex-1">{body}</div>
        </div>
      )}
    </div>
  );
}

// Memoized so that during streaming only the bubble whose HumanRow set changed
// re-renders. The store keeps unchanged HumanRow object references across a
// flush (messageStore applies `[...messages]` but only replaces the one
// streaming row), so an element-wise reference compare on `rows` lets every
// other visible bubble bail — only the live HumanRow's bubble moves.
export const ItemBubble = memo(
  ItemBubbleImpl,
  (prev, next) =>
    prev.sessionId === next.sessionId &&
    prev.isRunning === next.isRunning &&
    prev.showRevert === next.showRevert &&
    prev.showRevertFiles === next.showRevertFiles &&
    prev.userAnchorK === next.userAnchorK &&
    prev.bubbleKey === next.bubbleKey &&
    prev.editingAnchor === next.editingAnchor &&
    prev.rows.length === next.rows.length &&
    prev.rows.every((r, i) => r === next.rows[i]),
);

function firstContentRow(group: HumanRow[]): HumanRow | undefined {
  return group.find((row) => !isCompactCutRow(row));
}

/**
 * Group consecutive rows for display: each user message is its own bubble;
 * consecutive non-user Items (live shells or sealed) coalesce into one assistant bubble
 * so process/output grouping still works across Item atoms.
 *
 * Compact replace is its own barrier (not pushed into the previous assistant
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
    if (!isHumanViewKind(row.kind)) continue;
    if (isCompactCutRow(row) || isJobExitReminderRow(row)) {
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

const LIST_LOADER_KEY = "__list_loader__";
const LIST_LOADER_HEIGHT = 40;
/** Trailing transient "compacting…" row (not a real buffer item). */
const COMPACTING_PENDING_KEY = "__compacting_pending__";
const COMPACTING_LINE_HEIGHT = 22;

/**
 * Stable virtual-item identity for a bubble: min(seq) in the group.
 */
export function bubbleIdentity(bubbles: HumanRow[][], index: number): string {
  const group = bubbles[index] ?? [];
  let min: number | undefined;
  for (const row of group) {
    if (min === undefined || row.seq < min) min = row.seq;
  }
  return min === undefined ? String(index) : String(min);
}

/** True when this user-detail anchor can file-revert (`k <= max` from snapshot). */
export function canRevertFiles(
  k: number,
  maxFileRevertK: number | null | undefined,
): boolean {
  return maxFileRevertK != null && k <= maxFileRevertK;
}

/** Find the virtual bubble + FoldCard ids for a live bash job's tool card. */
export function locateBashTool(
  bubbles: HumanRow[][],
  callId: string,
  sessionId: string,
): { bubbleIndex: number; foldIds: string[] } | null {
  for (let i = 0; i < bubbles.length; i++) {
    const rows = bubbles[i]!;
    const groups = groupNodes(rowsToNodes(rows));
    const bubbleKey = bubbleIdentity(bubbles, i);
    for (let gi = 0; gi < groups.length; gi++) {
      const grouped = groups[gi]!;
      for (const node of grouped.nodes) {
        if (node.kind !== "tool" || node.call.call_id !== callId) continue;
        const foldIds = [`${sessionId}:${bubbleKey}:tool:${callId}`];
        if (grouped.type === "process") {
          foldIds.unshift(`${sessionId}:${bubbleKey}:process:${gi}`);
        }
        return { bubbleIndex: i, foldIds };
      }
    }
  }
  return null;
}

function bashCallSelector(callId: string): string {
  const escaped =
    typeof CSS !== "undefined" && typeof CSS.escape === "function"
      ? CSS.escape(callId)
      : callId.replace(/\\/g, "\\\\").replace(/"/g, '\\"');
  return `[data-bash-call-id="${escaped}"]`;
}

interface MessageListProps {
  messages: HumanRow[];
  loadingHistory: boolean;
  canLoadMore: boolean;
  onLoadMore: () => void;
  userDetailBefore: number;
  isRunning: boolean;
  scrollRef: RefObject<HTMLDivElement | null>;
  sessionId: string;
  /** Highest user-detail k with a nonempty file patch; null hides Revert files. */
  maxFileRevertK?: number | null;
  /** Human stick intent: true until the user scrolls up. */
  onStickChange?: (stickToEnd: boolean) => void;
  jumpToEndRef?: RefObject<(() => void) | null>;
  revealBashRef?: RefObject<((callId: string) => void) | null>;
  editingAnchor?: EditingUserAnchor | null;
  onEditAnchor?: (anchor: EditingUserAnchor) => void;
  onDismissEdit?: () => void;
  miniPhase?: "idle" | "entering" | "visible" | "exiting";
  onMiniAnimationEnd?: () => void;
}

export const MessageList = memo(function MessageList({
  messages,
  loadingHistory,
  canLoadMore,
  onLoadMore,
  userDetailBefore,
  isRunning,
  scrollRef,
  sessionId,
  maxFileRevertK = null,
  onStickChange,
  jumpToEndRef,
  revealBashRef,
  editingAnchor,
  onEditAnchor = () => {},
  onDismissEdit = () => {},
  miniPhase = "idle",
  onMiniAnimationEnd = () => {},
}: MessageListProps) {
  const bubbles = useMemo(() => groupRowsForBubbles(messages), [messages]);
  // Transient "compacting now" phase: manual compaction surfaces via `compacting`
  // (exclusive lease), auto via `turnPhase === "compacting"`. The real checkpoint
  // item replaces the pending row with a CompactCutMark on success.
  const compactingNow = useTurnStore((s) => {
    const t = s.byId.get(sessionId);
    return (t?.compacting ?? false) || t?.turnPhase === "compacting";
  });
  const loader = canLoadMore ? 1 : 0;
  const count = loader + bubbles.length + (compactingNow ? 1 : 0);

  const [bottomPad, setBottomPad] = useState(0);
  const [stickToEnd, setStickToEnd] = useState(true);
  const onStickChangeRef = useRef(onStickChange);
  onStickChangeRef.current = onStickChange;

  useEffect(() => {
    const measure = () => {
      const el = scrollRef.current;
      if (!el) return;
      const h = el.getBoundingClientRect().height;
      setBottomPad(h > 0 ? h / 2 : 0);
    };
    measure();
    const raf = requestAnimationFrame(measure);
    const ro = new ResizeObserver(measure);
    const el = scrollRef.current;
    if (el) ro.observe(el);
    return () => {
      cancelAnimationFrame(raf);
      ro.disconnect();
    };
  }, [scrollRef]);

  const getItemKey = useCallback(
    (index: number) => {
      if (loader && index === 0) return LIST_LOADER_KEY;
      const i = index - loader;
      if (i >= bubbles.length) return COMPACTING_PENDING_KEY;
      return bubbleIdentity(bubbles, i);
    },
    [bubbles, loader],
  );

  const estimateSize = useCallback(
    (index: number) => {
      if (loader && index === 0) return LIST_LOADER_HEIGHT;
      const i = index - loader;
      if (i >= bubbles.length) return COMPACTING_LINE_HEIGHT;
      const first = firstContentRow(bubbles[i] ?? []);
      if (!first) return 28;
      if (isHumanUserRow(first)) {
        return editingAnchor?.bubbleKey === bubbleIdentity(bubbles, i) ? 240 : 88;
      }
      return 240;
    },
    [bubbles, editingAnchor?.bubbleKey, loader],
  );

  const virtualizer = useVirtualizer({
    count,
    getScrollElement: () => scrollRef.current,
    estimateSize,
    overscan: 6,
    getItemKey,
    paddingEnd: bottomPad,
    anchorTo: "end",
    followOnAppend: stickToEnd,
  });

  const virtualItems = virtualizer.getVirtualItems();

  // Human stick intent: true until the user scrolls up. The stick flag is an
  // authoritative ref driven by gestures (see useStickToBottom); React state
  // (`stickToEnd`) is synced from it for the virtualizer + Latest button.
  const { setStick } = useStickToBottom({
    ref: scrollRef,
    active: true,
    initialStick: true,
    isAtEnd: () => virtualizer.isAtEnd(),
    onStickChange: useCallback(
      (next: boolean) => {
        setStickToEnd(next);
        onStickChangeRef.current?.(next);
      },
      [],
    ),
  });

  const pinToEnd = useCallback(() => {
    setStick(true);
    virtualizer.scrollToEnd();
  }, [setStick, virtualizer]);

  if (jumpToEndRef) jumpToEndRef.current = pinToEnd;

  const revealBash = useCallback(
    (callId: string) => {
      setStick(false);
      const located = locateBashTool(bubbles, callId, sessionId);
      if (!located) return;
      for (const foldId of located.foldIds) requestFoldCardOpen(foldId);
      virtualizer.scrollToIndex(loader + located.bubbleIndex, {
        align: "center",
      });
      const started = performance.now();
      const tick = () => {
        const el = document.querySelector(bashCallSelector(callId));
        if (el instanceof HTMLElement) {
          el.scrollIntoView({ block: "center", inline: "nearest" });
          el.classList.remove("bash-view-reveal");
          void el.offsetWidth;
          el.classList.add("bash-view-reveal");
          return;
        }
        if (performance.now() - started < 800) requestAnimationFrame(tick);
      };
      requestAnimationFrame(tick);
    },
    [bubbles, loader, sessionId, setStick, virtualizer],
  );
  if (revealBashRef) revealBashRef.current = revealBash;

  const totalSize = virtualizer.getTotalSize();
  useLayoutEffect(() => {
    if (!stickToEnd) return;
    virtualizer.scrollToEnd();
  }, [stickToEnd, totalSize, count, virtualizer]);

  useEffect(() => {
    if (!canLoadMore || loadingHistory) return;
    if (virtualItems.some((item) => item.index === 0)) {
      onLoadMore();
    }
  }, [canLoadMore, loadingHistory, onLoadMore, virtualItems]);

  const itemStyle = (start: number): CSSProperties => ({
    position: "absolute",
    top: 0,
    left: 0,
    width: "100%",
    transform: `translateY(${start}px)`,
  });

  return (
    <div data-testid="message-list">
      {count > 0 && (
        <div
          style={{
            height: `${virtualizer.getTotalSize()}px`,
            width: "100%",
            position: "relative",
          }}
        >
          {virtualItems.map((virtualItem) => {
            if (loader && virtualItem.index === 0) {
              return (
                <div
                  key={virtualItem.key}
                  data-index={0}
                  style={{
                    ...itemStyle(virtualItem.start),
                    height: LIST_LOADER_HEIGHT,
                  }}
                  aria-busy={loadingHistory}
                  aria-label={loadingHistory ? "Loading earlier items" : undefined}
                />
              );
            }

            const bubbleIndex = virtualItem.index - loader;
            if (bubbleIndex >= bubbles.length) {
              return (
                <div
                  key={virtualItem.key}
                  data-index={virtualItem.index}
                  ref={virtualizer.measureElement}
                  style={itemStyle(virtualItem.start)}
                >
                  <CompactingMark />
                </div>
              );
            }
            const group = bubbles[bubbleIndex];
            if (!group) return null;

            const cutOnly = group.every(isCompactCutRow);
            const first = firstContentRow(group);
            const firstIdx = first ? messages.indexOf(first) : -1;
            const sealed = first != null && first.seq >= 0;
            const isUser = first != null && isHumanUserRow(first);
            const showRevert = sealed && isUser && firstIdx >= 0;
            const userAnchorK = showRevert
              ? deriveUserAnchorK(messages, firstIdx, userDetailBefore)
              : undefined;
            const showRevertFiles =
              userAnchorK !== undefined &&
              canRevertFiles(userAnchorK, maxFileRevertK);
            const bubbleKey = bubbleIdentity(bubbles, bubbleIndex);

            return (
              <div
                key={virtualItem.key}
                data-index={virtualItem.index}
                ref={virtualizer.measureElement}
                style={itemStyle(virtualItem.start)}
              >
                {cutOnly
                  ? group.map((cut) => (
                      <CompactCutMark key={projectionRowKey(cut)} />
                    ))
                  : (
                  <ItemBubble
                    rows={group}
                    userAnchorK={userAnchorK}
                    showRevert={showRevert}
                    showRevertFiles={showRevertFiles}
                    isRunning={isRunning}
                    sessionId={sessionId}
                    bubbleKey={bubbleKey}
                    editingAnchor={editingAnchor ?? null}
                    onEditAnchor={onEditAnchor}
                    onDismissEdit={onDismissEdit}
                    miniPhase={miniPhase}
                    onMiniAnimationEnd={onMiniAnimationEnd}
                  />
                )}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
});
import { BrainIcon, WrenchIcon } from "@phosphor-icons/react";
import { memo, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type CSSProperties, type RefObject } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";

import type { ChatRow } from "../api/adapter";
import {
  deriveUserAnchorK,
  extractBufferIndex,
  isChatUserMessage,
  isCompactCutRow,
  isFunctionCall,
  isFunctionCallOutput,
  isMessageItem,
  isReasoningItem,
  isSystemReminderItem,
  isUserMessage,
  itemPlainText,
  projectionRowKey,
} from "../api/adapter";
import type {
  FunctionCallItem,
  FunctionCallOutputItem,
  Item,
} from "../api/types";
import { AgentMarkdown } from "./AgentMarkdown";
import { FoldCard } from "./FoldCard";
import { useMessageStore } from "../stores/messageStore";
import { useTurnStore } from "../stores/turnStore";
import { useSessionStore } from "../stores/sessionStore";
import { useEditorStore } from "../stores/editorStore";
import { PermissionCard } from "./PermissionModal";
import { MessageHistorySkeleton } from "./ui/Skeleton";
import { ToolCallCard } from "./ToolCallCard";
import { isToolCallLive, processGroupStreaming } from "./toolCallStatus";

type RenderNode =
  | { kind: "text"; text: string; key: string; streaming: boolean; incomplete?: boolean }
  | { kind: "reasoning"; text: string; key: string; streaming: boolean; incomplete?: boolean }
  | { kind: "compact_cut"; key: string; streaming: boolean }
  | {
      kind: "tool";
      call: FunctionCallItem;
      output?: FunctionCallOutputItem;
      key: string;
      streaming: boolean;
    }
  | { kind: "unknown"; type: string; key: string; streaming: boolean };

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

function outputsByCallId(items: Item[]): Map<string, FunctionCallOutputItem> {
  const map = new Map<string, FunctionCallOutputItem>();
  for (const item of items) {
    if (isFunctionCallOutput(item)) {
      map.set(item.call_id, item);
    }
  }
  return map;
}

/** Flatten ChatRows into render nodes; pair function_call + output by call_id. Only reads `row.item`. */
export function rowsToNodes(rows: ChatRow[], turnActive = false): RenderNode[] {
  const items = rows.map((r) => r.item);
  const outputs = outputsByCallId(items);
  // Per-output streaming state, so a tool node's flag reflects its result row too.
  const streamingByCallId = new Map<string, boolean>();
  for (const row of rows) {
    if (isFunctionCallOutput(row.item)) {
      streamingByCallId.set(row.item.call_id, row.streaming === true);
    }
  }
  const nodes: RenderNode[] = [];

  for (const row of rows) {
    const item = row.item;
    const streaming = row.streaming === true;
    const key = projectionRowKey(row);
    if (isCompactCutRow(row)) {
      nodes.push({ kind: "compact_cut", key, streaming: false });
      continue;
    }
    if (isFunctionCallOutput(item)) {
      // Rendered with matching function_call
      continue;
    }
    if (isFunctionCall(item)) {
      const output = outputs.get(item.call_id);
      nodes.push({
        kind: "tool",
        call: item,
        output,
        key,
        streaming: isToolCallLive(
          !!output,
          streaming,
          turnActive,
          streamingByCallId.get(item.call_id) === true,
        ),
      });
      continue;
    }
    if (isReasoningItem(item)) {
      const text = itemPlainText(item);
      if (text) {
        nodes.push({
          kind: "reasoning",
          text,
          key,
          streaming,
          incomplete: item.status === "incomplete",
        });
      }
      continue;
    }
    if (isMessageItem(item)) {
      const text = itemPlainText(item);
      if (text) {
        nodes.push({
          kind: "text",
          text,
          key,
          streaming,
          incomplete: item.status === "incomplete",
        });
      }
      continue;
    }
    nodes.push({
      kind: "unknown",
      type: String(item.type),
      key,
      streaming,
    });
  }

  return nodes;
}

type NodeGroup = { type: "process" | "output" | "cut"; nodes: RenderNode[] };

export function groupNodes(nodes: RenderNode[]): NodeGroup[] {
  const groups: NodeGroup[] = [];
  let current: NodeGroup | null = null;

  for (const node of nodes) {
    if (node.kind === "compact_cut") {
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
      return (
        <ToolCallCard
          call={node.call}
          output={node.output}
          streaming={streaming}
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
    case "unknown":
      return (
        <div className="text-xs text-(--_dk-text-disabled) italic pl-(--_dk-indent-card-head) pr-2 py-1">
          [{node.type}]
        </div>
      );
  }
}

export function ProcessGroup({
  nodes,
  streaming,
  sessionId,
  bubbleKey,
  groupIndex,
}: {
  nodes: RenderNode[];
  streaming: boolean;
  sessionId?: string;
  /** Stable bubble identity, used to namespace this group's FoldCard state. */
  bubbleKey?: string;
  /** Index of this process group within its bubble (for a unique FoldCard id). */
  groupIndex: number;
}) {
  const project = useSessionStore((s) => s.project);
  const openFile = useEditorStore((s) => s.openFile);

  const reasoningCount = nodes.filter((n) => n.kind === "reasoning").length;
  const toolCount = nodes.filter((n) => n.kind === "tool").length;
  const labels: string[] = [];
  if (reasoningCount > 0) labels.push(`${reasoningCount} reasoning`);
  if (toolCount > 0)
    labels.push(`${toolCount} tool call${toolCount !== 1 ? "s" : ""}`);
  const summary = labels.join(" + ") || "Process";

  return (
    <FoldCard
      id={
        bubbleKey && sessionId
          ? `${sessionId}:${bubbleKey}:process:${groupIndex}`
          : undefined
      }
      icon={
        <>
          {reasoningCount > 0 && (
            <BrainIcon size={13} aria-hidden className="shrink-0 text-(--_dk-text-muted)" />
          )}
          {toolCount > 0 && (
            <WrenchIcon size={13} aria-hidden className="shrink-0 text-(--_dk-amber-500)" />
          )}
        </>
      }
      label={summary}
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
  showRevertFiles = false,
  isRunning,
  sessionId,
  bubbleKey,
}: {
  rows: ChatRow[];
  userAnchorK?: number;
  showRevert: boolean;
  isRunning: boolean;
  sessionId: string;
  /** Stable identity of this bubble (projection key of its first row). Namespaces
   *  child FoldCard open-state so it survives virtual-list remounts. */
  bubbleKey?: string;
  showRevertFiles?: boolean;
}) {
  const revertToUserAnchor = useMessageStore((s) => s.revertToUserAnchor);
  const revertFiles = useMessageStore((s) => s.revertFiles);
  const first = rows.find((r) => !isCompactCutRow(r)) ?? rows[0];
  if (first != null && isSystemReminderItem(first.item)) {
    return <SystemReminderMark />;
  }
  const isUser = first != null && isChatUserMessage(first.item);
  const nodes = rowsToNodes(rows, isRunning);
  const streaming =
    rows.some((r) => r.streaming === true) || nodes.some((n) => n.streaming);
  const hasContent = nodes.length > 0 || !streaming;
  const groups = groupNodes(nodes);

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
        const hasTextAfter = groups
          .slice(gi + 1)
          .some((g) => g.type === "output");
        const groupLive = processGroupStreaming({
          hasTextAfter,
          turnActive: isRunning,
        });
        return (
          <ProcessGroup
            // Index within this bubble — stable as the group grows and across
            // live→seal (must NOT use row.id / first-node key; those remount).
            key={`process-${gi}`}
            nodes={group.nodes}
            streaming={groupLive}
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
        <div className="flex items-start gap-2">
          <span className="mt-[7px] h-1.5 w-1.5 shrink-0 rounded-full bg-(--_dk-accent-hover)" />
          <div className="min-w-0 flex-1">{body}</div>
        </div>
      ) : (
        <div className="flex items-start gap-2">
          <span className="mt-[7px] h-1.5 w-1.5 shrink-0 rounded-full bg-(--_dk-text-muted)" />
          <div className="min-w-0 flex-1">{body}</div>
        </div>
      )}
      {showRevert && isUser && userAnchorK !== undefined && (
        <div className="mt-1 flex justify-start gap-1 pl-[14px] opacity-80">
          <button
            type="button"
            onClick={() => revertToUserAnchor(sessionId, userAnchorK)}
            className="rounded px-1.5 py-0.5 text-dk-2xs text-(--_dk-text-muted) hover:bg-(--_dk-ix-bg-hover) hover:text-(--_dk-ix-fg-hover) disabled:cursor-not-allowed disabled:opacity-40"
            title="Revert transcript to here"
          >
            Revert to here
          </button>
          {showRevertFiles && (
            <button
              type="button"
              disabled={isRunning}
              onClick={() => revertFiles(sessionId, userAnchorK)}
              className="rounded px-1.5 py-0.5 text-dk-2xs text-(--_dk-text-muted) hover:bg-(--_dk-ix-bg-hover) hover:text-(--_dk-ix-fg-hover) disabled:cursor-not-allowed disabled:opacity-40"
              title="Revert only files changed by the agent since this message (OpenCode-style)"
            >
              Revert files
            </button>
          )}
        </div>
      )}
    </div>
  );
}

// Memoized so that during streaming only the bubble whose ChatRow set changed
// re-renders. The store keeps unchanged ChatRow object references across a
// flush (messageStore applies `[...messages]` but only replaces the one
// streaming row), so an element-wise reference compare on `rows` lets every
// other visible bubble bail — only the live ChatRow's bubble moves.
export const ItemBubble = memo(
  ItemBubbleImpl,
  (prev, next) =>
    prev.sessionId === next.sessionId &&
    prev.isRunning === next.isRunning &&
    prev.showRevert === next.showRevert &&
    prev.showRevertFiles === next.showRevertFiles &&
    prev.userAnchorK === next.userAnchorK &&
    prev.bubbleKey === next.bubbleKey &&
    prev.rows.length === next.rows.length &&
    prev.rows.every((r, i) => r === next.rows[i]),
);

function firstContentRow(group: ChatRow[]): ChatRow | undefined {
  return group.find((row) => !isCompactCutRow(row));
}

function splitLeadingCuts(group: ChatRow[]): {
  cutsBefore: ChatRow[];
  rows: ChatRow[];
} {
  let i = 0;
  while (i < group.length && isCompactCutRow(group[i]!)) i += 1;
  return { cutsBefore: group.slice(0, i), rows: group.slice(i) };
}

/**
 * Group consecutive rows for display: each user message is its own bubble;
 * consecutive non-user Items (live shells or sealed) coalesce into one assistant bubble
 * so process/output grouping still works across Item atoms.
 *
 * Compact cuts sit between items: leading cuts ride on the next bubble (between
 * bubbles); a cut between assistant atoms stays inside that bubble.
 */
export function groupRowsForBubbles(rows: ChatRow[]): ChatRow[][] {
  const groups: ChatRow[][] = [];
  let current: ChatRow[] = [];

  const flush = () => {
    if (current.length) {
      groups.push(current);
      current = [];
    }
  };

  for (const row of rows) {
    if (isCompactCutRow(row)) {
      current.push(row);
      continue;
    }
    if (isUserMessage(row.item)) {
      if (current.length > 0 && current.every(isCompactCutRow)) {
        groups.push([...current, row]);
        current = [];
      } else {
        flush();
        groups.push([row]);
      }
    } else {
      current.push(row);
    }
  }
  flush();
  return groups;
}

/** Sentinel key for the trailing permission card. */
export const LIST_FOOTER_KEY = "__list_footer__";

/**
 * Stable virtual-item identity for a bubble.
 *
 * Must NOT be `projectionRowKey(firstRow)`: `orderSealedBeforeTransient`
 * can change which row leads an assistant group when a later tool seals
 * first. A key flip remounts the bubble at `estimateSize`, which is what
 * made the whole list jump while output was still streaming.
 *
 * Assistant bubbles are keyed by the nearest preceding user *or* system-
 * reminder bubble. Reminders split the list (user-role rows) so skipping
 * them in lookback would give two assistant groups the same
 * `assistant-after:user:…` key. Compact cuts are not bubbles and do not
 * affect identity.
 */
export function bubbleIdentity(bubbles: ChatRow[][], index: number): string {
  const group = bubbles[index];
  const first = firstContentRow(group ?? []) ?? group?.[0];
  if (!first) return String(index);
  if (isCompactCutRow(first)) {
    return `compact:${projectionRowKey(first)}`;
  }
  if (isSystemReminderItem(first.item)) {
    return `notice:${projectionRowKey(first)}`;
  }
  if (isChatUserMessage(first.item)) {
    return `user:${projectionRowKey(first)}`;
  }
  for (let i = index - 1; i >= 0; i--) {
    const prev = firstContentRow(bubbles[i] ?? []) ?? bubbles[i]?.[0];
    if (!prev) continue;
    if (isSystemReminderItem(prev.item)) {
      return `assistant-after:notice:${projectionRowKey(prev)}`;
    }
    if (isChatUserMessage(prev.item)) {
      return `assistant-after:user:${projectionRowKey(prev)}`;
    }
  }
  return "assistant-lead";
}

/** True when this user-detail anchor can file-revert (`k <= max` from snapshot). */
export function canRevertFiles(
  k: number,
  maxFileRevertK: number | null | undefined,
): boolean {
  return maxFileRevertK != null && k <= maxFileRevertK;
}

interface MessageListProps {
  messages: ChatRow[];
  loadingHistory: boolean;
  canLoadMore: boolean;
  onLoadMore: () => void;
  userDetailBefore: number;
  isRunning: boolean;
  scrollRef: RefObject<HTMLDivElement | null>;
  sessionId: string;
  /** Highest user-detail k with a nonempty file patch; null hides Revert files. */
  maxFileRevertK?: number | null;
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
}: MessageListProps) {
  const pendingPermission = useTurnStore(
    (s) => s.byId.get(sessionId)?.pendingPermission ?? null,
  );
  const rawGrantPermission = useTurnStore((s) => s.grantPermission);
  const grantPermission = useCallback(
    (approved: boolean, always: boolean) => {
      rawGrantPermission(sessionId, approved, always);
    },
    [sessionId, rawGrantPermission],
  );

  // Cache the grouping on the messages reference. The store only swaps the
  // `messages` array on a stream flush, so between flushes the bubble array is
  // referentially stable and the virtualizer / ItemBubble props don't churn on
  // unrelated re-renders. This is the pre-gate O(N) cost — run once per real
  // data change, not per render.
  const bubbles = useMemo(() => groupRowsForBubbles(messages), [messages]);

  const headerRef = useRef<HTMLDivElement | null>(null);
  const [scrollMargin, setScrollMargin] = useState(0);
  const hasFooter = pendingPermission != null;
  const bindHeader = useCallback((node: HTMLDivElement | null) => {
    headerRef.current = node;
    setScrollMargin(node?.offsetHeight ?? 0);
  }, []);
  useLayoutEffect(() => {
    const node = headerRef.current;
    if (!node) return;
    const ro = new ResizeObserver(() => {
      setScrollMargin(node.offsetHeight);
    });
    ro.observe(node);
    return () => ro.disconnect();
  }, [canLoadMore, loadingHistory]);

  const getItemKey = useCallback(
    (index: number) => {
      if (index >= bubbles.length) return LIST_FOOTER_KEY;
      return bubbleIdentity(bubbles, index);
    },
    [bubbles],
  );

  const estimateSize = useCallback(
    (index: number) => {
      if (index >= bubbles.length) return 72;
      const first = firstContentRow(bubbles[index] ?? []);
      if (!first) return 28;
      if (isSystemReminderItem(first.item)) return 28;
      if (isChatUserMessage(first.item)) return 88;
      return 240;
    },
    [bubbles],
  );

  // Unpin when the user scrolls away from the real bottom (spacer included).
  const [stickToEnd, setStickToEnd] = useState(true);
  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const onScroll = () => {
      const atEnd = el.scrollHeight - el.scrollTop - el.clientHeight <= 1;
      setStickToEnd((prev) => (prev === atEnd ? prev : atEnd));
    };
    el.addEventListener("scroll", onScroll, { passive: true });
    onScroll();
    return () => el.removeEventListener("scroll", onScroll);
  }, [scrollRef]);

  const virtualizer = useVirtualizer({
    count: messages.length === 0 ? 0 : bubbles.length + (hasFooter ? 1 : 0),
    getScrollElement: () => scrollRef.current,
    estimateSize,
    overscan: 6,
    getItemKey,
    scrollMargin,
    anchorTo: "end",
    followOnAppend: stickToEnd,
  });

  const virtualItems = virtualizer.getVirtualItems();
  const pinnedOnMount = useRef(false);
  useLayoutEffect(() => {
    if (pinnedOnMount.current || bubbles.length === 0) return;
    pinnedOnMount.current = true;
    virtualizer.scrollToEnd();
  }, [bubbles.length, virtualizer]);

  const itemStyle = (start: number): CSSProperties => ({
    position: "absolute",
    top: 0,
    left: 0,
    width: "100%",
    transform: `translateY(${start - virtualizer.options.scrollMargin}px)`,
  });

  return (
    <div
      className={messages.length === 0 ? "flex flex-1 flex-col" : undefined}
      data-testid="message-list"
    >
      {canLoadMore && (
        <div ref={bindHeader} className="shrink-0 p-2 text-center">
          {loadingHistory ? (
            <MessageHistorySkeleton />
          ) : (
            <button
              type="button"
              onClick={onLoadMore}
              className="text-xs text-(--_dk-accent-hover) hover:text-(--_dk-accent-hover)"
            >
              Load earlier items
            </button>
          )}
        </div>
      )}
      {messages.length === 0 ? (
        <div className="agent-empty-state">
          <p className="agent-empty-state-title">
            Send a message to start the agent.
          </p>
          <p className="agent-empty-state-hint">
            Enter to send · Shift+Enter for newline
          </p>
        </div>
      ) : (
        <div
          style={{
            height: `${virtualizer.getTotalSize()}px`,
            width: "100%",
            position: "relative",
          }}
        >
          {virtualItems.map((virtualItem) => {
            if (virtualItem.index >= bubbles.length) {
              if (!pendingPermission) return null;
              return (
                <div
                  key={virtualItem.key}
                  data-index={virtualItem.index}
                  ref={virtualizer.measureElement}
                  style={itemStyle(virtualItem.start)}
                >
                  <PermissionCard
                    tool={pendingPermission.tool}
                    ruleId={pendingPermission.rule_id}
                    summary={pendingPermission.summary}
                    requestId={pendingPermission.request_id}
                    onGrant={grantPermission}
                  />
                </div>
              );
            }

            const group = bubbles[virtualItem.index];
            if (!group) return null;

            const { cutsBefore, rows: contentRows } = splitLeadingCuts(group);
            const first = contentRows[0];
            const firstIdx = first ? messages.indexOf(first) : -1;
            // Sealed buffer rows only — optimistic live user shells have no buffer id.
            const sealed =
              first != null && extractBufferIndex(first.id) !== null;
            const isUser = first != null && isChatUserMessage(first.item);
            const showRevert = sealed && isUser && firstIdx >= 0;
            const userAnchorK = showRevert
              ? deriveUserAnchorK(messages, firstIdx, userDetailBefore)
              : undefined;
            const showRevertFiles =
              userAnchorK !== undefined &&
              canRevertFiles(userAnchorK, maxFileRevertK);
            const bubbleKey = bubbleIdentity(bubbles, virtualItem.index);

            return (
              <div
                key={virtualItem.key}
                data-index={virtualItem.index}
                ref={virtualizer.measureElement}
                style={itemStyle(virtualItem.start)}
              >
                {cutsBefore.map((cut) => (
                  <CompactCutMark key={projectionRowKey(cut)} />
                ))}
                {contentRows.length > 0 && (
                  <ItemBubble
                    rows={contentRows}
                    userAnchorK={userAnchorK}
                    showRevert={showRevert}
                    showRevertFiles={showRevertFiles}
                    isRunning={isRunning}
                    sessionId={sessionId}
                    bubbleKey={bubbleKey}
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
import { memo, useMemo, type CSSProperties, type RefObject } from "react";

import type { HumanRow, PendingMessage } from "../api/types";
import { canRevertFiles, projectBubbles } from "../lib/transcriptProjection";
import { useMessageStore } from "../stores/messageStore";
import { useTurnStore } from "../stores/turnStore";
import { CompactingMark, TranscriptMarkForRow } from "./transcriptMarks";
import { ItemBubble, type EditingUserAnchor } from "./transcript/ItemBubble";
import { PendingQueueBubble } from "./transcript/PendingQueueBubble";
import { useBottomPad, useBottomPadMotion } from "./transcript/useBottomPad";
import {
  HISTORY_LOADER_HEIGHT,
  useTranscriptViewport,
} from "./transcript/useTranscriptViewport";

export type { EditingUserAnchor };

/** Empty fallback so store selectors never allocate per snapshot. */
const EMPTY_PENDING: PendingMessage[] = [];

interface MessageListProps {
  messages: HumanRow[];
  loadingHistory: boolean;
  canLoadMore: boolean;
  onLoadMore: () => void;
  isRunning: boolean;
  scrollRef: RefObject<HTMLDivElement | null>;
  sessionId: string;
  /** Highest file-snapshot stem with a nonempty patch; null hides Revert files. */
  maxFileRevertSeq?: number | null;
  /** Human stick intent: true until the user scrolls up. */
  onStickChange?: (stickToEnd: boolean) => void;
  jumpToEndRef?: RefObject<(() => void) | null>;
  revealBashRef?: RefObject<((callId: string) => void) | null>;
  revealSeqRef?: RefObject<((seq: number) => void) | null>;
  editingAnchor?: EditingUserAnchor | null;
  onEditAnchor?: (anchor: EditingUserAnchor) => void;
  onDismissEdit?: () => void;
  miniPhase?: "idle" | "entering" | "visible" | "exiting";
  onMiniAnimationEnd?: () => void;
  /** Composer dock is collapsed: its cards no longer float over the tail, so the
   *  list is free to give back most of the bottom pad (see PAD_COLLAPSED). */
  composerCollapsed?: boolean;
  /** Hard read-only boundary: user bubbles get no MiniChat/revert/replay and no
   *  text cursor. Not merely a noop handler — the writable affordances are not
   *  built at all. */
  readOnly?: boolean;
}

export const MessageList = memo(function MessageList({
  messages,
  loadingHistory,
  canLoadMore,
  onLoadMore,
  isRunning,
  scrollRef,
  sessionId,
  maxFileRevertSeq = null,
  onStickChange,
  jumpToEndRef,
  revealBashRef,
  revealSeqRef,
  editingAnchor,
  onEditAnchor = () => {},
  onDismissEdit = () => {},
  miniPhase = "idle",
  onMiniAnimationEnd = () => {},
  composerCollapsed = false,
  readOnly = false,
}: MessageListProps) {
  const bubbles = useMemo(() => projectBubbles(messages), [messages]);
  // Transient "compacting now" line: `compacting` is set on started and cleared
  // on succeeded/failed. Do not key off `turnPhase`, which can stay compacting
  // after the checkpoint lands.
  const compactingNow = useTurnStore(
    (s) => s.byId.get(sessionId)?.compacting ?? false,
  );
  // Plan-execution marks name the plan the button launched; the persisted row
  // carries only the prompt text, so the path comes from the session pointer.
  const activePlanPath = useTurnStore(
    (s) => s.byId.get(sessionId)?.activePlanPath ?? null,
  );
  // Queued batch: the in-flight bubble plus whether the server still holds it
  // (only then is recalling it honest).
  const pendingQueue = useMessageStore(
    (s) => s.bySession.get(sessionId)?.pendingQueue ?? null,
  );
  const pendingMessages = useTurnStore(
    (s) => s.byId.get(sessionId)?.pendingMessages ?? EMPTY_PENDING,
  );
  const recallPending = useTurnStore((s) => s.recallPendingMessages);
  // The durable row that takes over from the in-flight bubble lands exactly
  // where the bubble already was, so there is no travel to animate: what the
  // "sending" moment needs is the message settling in instead of snapping. The
  // store names the row it sealed by (its seq) in the same update that drops the
  // bubble, so the settle fires wherever that row landed — a batch injected at a
  // mid-turn seam is not the last bubble — and only while it is fresh.
  const landedQueueSeq = useMessageStore(
    (s) => s.bySession.get(sessionId)?.landedQueueSeq ?? null,
  );
  const clearLandedQueueSeq = useMessageStore((s) => s.clearLandedQueueSeq);
  const compactingRows = compactingNow ? 1 : 0;
  const queueRows = pendingQueue ? 1 : 0;

  const pad = useBottomPad({ scrollRef, composerCollapsed });
  const view = useTranscriptViewport({
    bubbles,
    compactingRows,
    queueRows,
    queueText: pendingQueue?.joined ?? null,
    queueImageCount: pendingQueue?.images.length ?? 0,
    canLoadMore,
    loadingHistory,
    onLoadMore,
    scrollRef,
    sessionId,
    paddingEnd: pad.bottomPad,
    padGrewRef: pad.padGrewRef,
    editingBubbleKey: editingAnchor?.bubbleKey,
    onStickChange,
    jumpToEndRef,
    revealBashRef,
    revealSeqRef,
  });
  useBottomPadMotion({
    scrollRef,
    composerCollapsed,
    padCollapsed: pad.padCollapsed,
    setPadCollapsed: pad.setPadCollapsed,
    padGrewRef: pad.padGrewRef,
    padGone: pad.padGone,
    virtualizer: view.virtualizer,
    setStick: view.setStick,
  });

  const itemStyle = (start: number): CSSProperties => ({
    position: "absolute",
    top: 0,
    left: 0,
    width: "100%",
    transform: `translateY(${start}px)`,
  });

  const showList =
    bubbles.length + compactingRows + queueRows > 0 || canLoadMore;

  return (
    <div data-testid="message-list">
      {showList && (
        <div
          style={{
            height: `${view.totalSize}px`,
            width: "100%",
            position: "relative",
          }}
        >
          {canLoadMore ? (
            <div
              aria-busy={loadingHistory}
              aria-label={loadingHistory ? "Loading earlier items" : undefined}
              style={{
                position: "absolute",
                top: 0,
                left: 0,
                width: "100%",
                height: HISTORY_LOADER_HEIGHT,
              }}
            />
          ) : null}
          {view.virtualItems.map((virtualItem) => {
            if (
              pendingQueue &&
              virtualItem.index === bubbles.length + compactingRows
            ) {
              return (
                <div
                  key={virtualItem.key}
                  data-index={virtualItem.index}
                  ref={view.virtualizer.measureElement}
                  style={itemStyle(virtualItem.start)}
                >
                  <PendingQueueBubble
                    text={pendingQueue.joined}
                    images={pendingQueue.images}
                    canRecall={!readOnly && pendingMessages.length > 0}
                    onRecall={() => void recallPending?.(sessionId)}
                  />
                </div>
              );
            }
            if (virtualItem.index >= bubbles.length) {
              return (
                <div
                  key={virtualItem.key}
                  data-index={virtualItem.index}
                  ref={view.virtualizer.measureElement}
                  style={itemStyle(virtualItem.start)}
                >
                  <CompactingMark />
                </div>
              );
            }
            const bubble = bubbles[virtualItem.index];
            if (!bubble) return null;

            const userSeq =
              bubble.isUser && bubble.first != null && bubble.first.seq >= 0
                ? bubble.first.seq
                : undefined;
            const showRevert = !readOnly && userSeq !== undefined;
            const showRevertFiles =
              userSeq !== undefined &&
              canRevertFiles(userSeq, maxFileRevertSeq);
            // The queue hands over to its durable row: "sending" — that row,
            // in the slot the bubble already occupied, settles in under the veil
            // instead of snapping to full opacity.
            const landedFromQueue =
              landedQueueSeq !== null &&
              bubble.first != null &&
              bubble.first.seq === landedQueueSeq;

            const item = bubble.markOnly ? (
              bubble.rows.map((cut) => (
                <TranscriptMarkForRow
                  key={String(cut.seq)}
                  row={cut}
                  planPath={activePlanPath}
                />
              ))
            ) : (
              <ItemBubble
                rows={bubble.rows}
                userSeq={userSeq}
                showRevert={showRevert}
                showRevertFiles={showRevertFiles}
                readOnly={readOnly}
                isRunning={isRunning}
                followedByUser={bubble.followedByUser}
                sessionId={sessionId}
                bubbleKey={bubble.key}
                editingAnchor={editingAnchor ?? null}
                onEditAnchor={onEditAnchor}
                onDismissEdit={onDismissEdit}
                miniPhase={miniPhase}
                onMiniAnimationEnd={onMiniAnimationEnd}
              />
            );

            return (
              <div
                key={virtualItem.key}
                data-index={virtualItem.index}
                data-seq-hit={bubble.rows.map((row) => row.seq).join(" ")}
                ref={view.virtualizer.measureElement}
                style={itemStyle(virtualItem.start)}
              >
                {/* The positioned wrapper owns its own translateY, so the
                    settle animates a child instead of clobbering it. */}
                {landedFromQueue ? (
                  <div
                    className="queued-bubble-land"
                    onAnimationEnd={(event) => {
                      // Nested animations bubble: only the settle clears it, and
                      // only once it has played, so a remount cannot replay it.
                      if (event.target !== event.currentTarget) return;
                      clearLandedQueueSeq(sessionId);
                    }}
                  >
                    {item}
                  </div>
                ) : (
                  item
                )}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
});

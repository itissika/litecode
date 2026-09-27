import { memo, useEffect, useRef, useState, type ReactNode } from "react";

import { isHumanUserRow, isTranscriptMarkRow } from "../../api/adapter";
import type { HumanRow } from "../../api/types";
import {
  groupNodes,
  processGroupHasTerminalStop,
  rowsToNodes,
} from "../../lib/transcriptProjection";
import { useSessionStore } from "../../stores/sessionStore";
import { useTurnStore } from "../../stores/turnStore";
import { MiniChatInput, type MiniChatInputSettings } from "../MiniChatInput";
import { processGroupAutoOpen } from "../toolCallStatus";
import { NodeView, ProcessGroup } from "./NodeView";

export interface EditingUserAnchor {
  bubbleKey: string;
  userAnchorK: number;
  draft: string;
  images?: string[];
  settings: MiniChatInputSettings;
  /** Height of the clicked bubble, used as the mini chat's expand-from value. */
  startHeight: number;
}

/**
 * Wrapper that animates the mini chat in by expanding its height from the
 * clicked bubble's height to its natural height, then hands off to auto-height
 * (so the textarea's own sizing takes over with no jitter). Width stays 100%
 * the whole time — the old width-based animation made the textarea measure
 * `scrollHeight` at the narrow start width, freezing a ~2x-tall height.
 */
function MiniChatPanel({
  startHeight,
  miniPhase,
  onDone,
  children,
}: {
  startHeight: number;
  miniPhase: "idle" | "entering" | "visible" | "exiting";
  onDone: () => void;
  children: ReactNode;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const measuredRef = useRef(false);
  const [height, setHeight] = useState(startHeight);
  const [opacity, setOpacity] = useState(0);

  useEffect(() => {
    if (miniPhase !== "entering" || measuredRef.current) return;
    measuredRef.current = true;
    const el = ref.current;
    if (!el) return;
    // The textarea is already sized at full width (child layout effect runs
    // first), so scrollHeight is the mini chat's natural height.
    const natural = el.scrollHeight;
    // Defer to the next frame so the browser paints the start height first,
    // then the height/opacity transition runs.
    requestAnimationFrame(() => {
      setHeight(natural);
      setOpacity(1);
    });
  }, [miniPhase]);

  return (
    <div
      ref={ref}
      className={`relative z-10 w-full origin-bottom ${
        miniPhase === "entering" || miniPhase === "exiting"
          ? "overflow-hidden"
          : "overflow-visible"
      } ${miniPhase === "entering" ? "mini-chat-enter" : ""} ${
        miniPhase === "exiting" ? "animate-mini-chat-exit" : ""
      }`}
      style={miniPhase === "entering" ? { height, opacity } : undefined}
      onTransitionEnd={(e) => {
        if (e.target !== e.currentTarget) return;
        if (miniPhase !== "entering") return;
        // Height always changes unless the mini chat is exactly as tall as the
        // bubble; opacity always changes (0 -> 1), so either one finishing is a
        // reliable "enter done" signal.
        if (e.propertyName === "height" || e.propertyName === "opacity") {
          onDone();
        }
      }}
      onAnimationEnd={(e) => {
        if (e.target !== e.currentTarget) return;
        if (miniPhase === "exiting") onDone();
      }}
    >
      {children}
    </div>
  );
}

/** Bubble for a contiguous run of rows that share the same speaker side. */
function ItemBubbleImpl({
  rows,
  userAnchorK,
  showRevert,
  readOnly,
  sessionId,
  bubbleKey,
  editingAnchor,
  followedByUser,
  onEditAnchor,
  onDismissEdit,
  miniPhase,
  onMiniAnimationEnd,
}: {
  rows: HumanRow[];
  userAnchorK?: number;
  showRevert: boolean;
  readOnly: boolean;
  isRunning: boolean;
  sessionId: string;
  /** Stable identity of this bubble (`min(seq)`). Namespaces
   *  child FoldCard open-state so it survives virtual-list remounts. */
  bubbleKey?: string;
  showRevertFiles?: boolean;
  /** The next bubble is a user message, so the last process group is complete. */
  followedByUser: boolean;
  editingAnchor: EditingUserAnchor | null;
  onEditAnchor: (anchor: EditingUserAnchor) => void;
  onDismissEdit: () => void;
  miniPhase: "idle" | "entering" | "visible" | "exiting";
  onMiniAnimationEnd: () => void;
}) {
  const sessionSettings = useSessionStore((s) => s.byId.get(sessionId));
  const replayFromAnchor = useTurnStore((s) => s.replayFromAnchor);
  const replaying = useTurnStore(
    (s) => s.byId.get(sessionId)?.replaying ?? false,
  );
  const first = rows.find((r) => !isTranscriptMarkRow(r)) ?? rows[0];
  const isUser = first != null && isHumanUserRow(first);
  const nodes = rowsToNodes(rows);
  const streaming =
    rows.some((r) => r.state === "in_progress") || nodes.some((n) => n.streaming);
  const hasContent = nodes.length > 0 || !streaming;
  const groups = groupNodes(nodes);
  const userText = nodes.find((node) => node.kind === "text")?.text ?? "";
  const userImages =
    nodes.find((node) => node.kind === "images")?.refs ?? [];
  const editing =
    !readOnly &&
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
          <NodeView
            key={n.key}
            node={n}
            sessionId={sessionId}
            bubbleKey={bubbleKey}
          />
        ));
      }
      if (group.type === "process") {
        const groupLive = group.nodes.some(
          (n) => n.kind !== "compact_cut" && n.live,
        );
        const followedByMessage =
          groups[gi + 1]?.type === "output" || followedByUser;
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
          citations={!isUser}
        />
      ));
    })
  );

  return (
    <div className={isUser ? "py-4" : "py-2"}>
      {isUser ? (
        editing ? (
          <MiniChatPanel
            startHeight={editingAnchor.startHeight}
            miniPhase={miniPhase}
            onDone={onMiniAnimationEnd}
          >
            <MiniChatInput
              sessionId={sessionId}
              draft={editingAnchor.draft}
              images={editingAnchor.images ?? userImages}
              settings={editingAnchor.settings}
              disabled={replaying}
              onDismiss={onDismissEdit}
              onChange={(draft, settings) => {
                onEditAnchor({ ...editingAnchor, draft, settings });
              }}
              onSubmit={(input, settings) => {
                onDismissEdit();
                void replayFromAnchor(
                  sessionId,
                  userAnchorK,
                  input,
                  settings,
                  editingAnchor.images ?? userImages,
                );
              }}
            />
          </MiniChatPanel>
        ) : (
          <div
            data-user-message-bubble
            className={`flex items-start gap-2 ${readOnly ? "" : "cursor-text"}`}
            onClick={(event) => {
              if (
                readOnly ||
                !showRevert ||
                userAnchorK === undefined ||
                !bubbleKey ||
                editing
              )
                return;
              onEditAnchor({
                bubbleKey,
                userAnchorK,
                draft: userText,
                images: userImages,
                settings: {
                  primaryId: sessionSettings?.activePrimary ?? "default",
                  modelId: sessionSettings?.modelId ?? "",
                  thinkingTier: sessionSettings?.thinkingTier ?? "medium",
                  contextMode: sessionSettings?.contextMode ?? "standard",
                },
                startHeight: event.currentTarget.getBoundingClientRect().height,
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
    prev.readOnly === next.readOnly &&
    prev.showRevertFiles === next.showRevertFiles &&
    prev.userAnchorK === next.userAnchorK &&
    prev.bubbleKey === next.bubbleKey &&
    prev.followedByUser === next.followedByUser &&
    prev.editingAnchor === next.editingAnchor &&
    prev.rows.length === next.rows.length &&
    prev.rows.every((r, i) => r === next.rows[i]),
);

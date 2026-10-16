import { useRef } from "react";

import { AgentChatInput } from "./AgentChatInput";
import { LlmReconnectBubble } from "./LlmReconnectBubble";
import { MessageList } from "./MessageList";
import type { RevealSeq } from "./transcript/transcriptScrollGlide";
import {
  TranscriptTopBlur,
  TranscriptVeil,
  usePendingRevealScroll,
  useTopBlur,
  useTranscriptFeed,
} from "./transcript/TranscriptBody";
import { SessionStatusLine } from "./SessionStatusLine";

/**
 * The read-only transcript region for a child session.
 *
 * It reuses the FULL transcript stack (message projection, `MessageList`
 * virtualizer, history paging, scroll and FoldCards) but passes `readOnly` to
 * `MessageList`, so a user bubble can never open MiniChat / revert / replay.
 *
 * The control strip pinned at the bottom is the DERIVED subagent view: the same
 * `SessionStatusLine` + `AgentChatInput` the writable shell mounts, both in
 * their `variant="subagent"` form. Session-row knobs (model / thinking tier /
 * context mode) and the usage ring stay editable — they are honored by the
 * child's own next turn — while the agent picker, composer, send/cancel,
 * permission card, Workers roster and the human-owned actions (terminal Kill,
 * plan execution, compaction) are not rendered at all.
 *
 * It deliberately owns no connection subscription: the hosting panel
 * (`SubagentReadOnlyPanel`, or `AgentPanel` when it has classified the session
 * as a child/unknown) owns the single subscribe/teardown lifecycle, which keeps
 * the non-refcounted `ensureSubscribe` from being armed twice.
 */
export function SubagentReadOnlyContent({
  sessionId,
  isActive = true,
}: {
  sessionId: string;
  isActive?: boolean;
}) {
  const { messages, loadingHistory, canLoadMore, isRunning, loadMoreHistory } =
    useTranscriptFeed(sessionId);
  const listRef = useRef<HTMLDivElement>(null);
  const revealSeqRef = useRef<RevealSeq | null>(null);
  const { blurOpacity, onScroll } = useTopBlur(listRef);

  // Same reveal contract as the writable shell (Search hit → load the seq, then
  // scroll it into view) so a read-only panel can locate a search result.
  usePendingRevealScroll(sessionId, revealSeqRef);

  return (
    <div className="relative flex min-h-0 flex-1 flex-col">
      {/* Same three-layer structure as the writable transcript: a non-scrolling
          frame carries the inset, the middle element is the virtualizer's
          scroll container, the inner column is the reading measure. */}
      <div className="flex min-h-0 flex-1 flex-col bg-(--_dk-editor) px-4 pt-4">
        <div
          ref={listRef}
          onScroll={onScroll}
          className="min-h-0 flex-1 overflow-y-auto bg-(--_dk-editor) [container-type:size]"
        >
          <div className="mx-auto flex w-full max-w-[var(--_dk-prose-measure)] flex-col bg-(--_dk-editor)">
            <MessageList
              key={sessionId}
              messages={messages}
              loadingHistory={loadingHistory}
              canLoadMore={canLoadMore}
              onLoadMore={loadMoreHistory}
              isRunning={isRunning}
              scrollRef={listRef}
              sessionId={sessionId}
              readOnly
              revealSeqRef={revealSeqRef}
            />
          </div>
        </div>
      </div>
      <TranscriptVeil isActive={isActive} />
      <TranscriptTopBlur opacity={blurOpacity} />

      {/* Derived subagent controls — floats over the transcript exactly like the
          writable `ComposerDock`, with every human-composition surface removed.
          Same flex clamp: the dock box spans the pane, its column is bottom-
          aligned, and the status panel is the only shrinkable item. */}
      <div className="pointer-events-none absolute inset-0 z-10 flex min-h-0 flex-col justify-end px-4 pb-4">
        <div className="pointer-events-auto mx-auto flex min-h-0 w-full max-w-[var(--_dk-prose-measure)] flex-col gap-2">
          <LlmReconnectBubble sessionId={sessionId} allowRetry={false} />
          <SessionStatusLine sessionId={sessionId} variant="subagent" />
          <AgentChatInput
            key={sessionId}
            sessionId={sessionId}
            variant="subagent"
          />
        </div>
      </div>
    </div>
  );
}

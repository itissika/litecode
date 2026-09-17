import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from "react";

import {
  clearPendingReveal,
  getPendingReveal,
  subscribePendingReveal,
} from "../lib/sessionPanelNav";
import { displayMessages, useMessageStore } from "../stores/messageStore";
import { useTurnStore } from "../stores/turnStore";
import { MessageList } from "./MessageList";
import { ProgressiveBlur } from "./ProgressiveBlur";

/**
 * The read-only transcript region for a child session.
 *
 * It reuses the FULL transcript stack (message projection, `MessageList`
 * virtualizer, history paging, scroll and FoldCards) but passes `readOnly` to
 * `MessageList`, so a user bubble can never open MiniChat / revert / replay, and
 * it mounts NO composer, permission card, status line or model/agent controls.
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
  const messages = useMessageStore((s) =>
    displayMessages(s.bySession.get(sessionId)),
  );
  const loadingHistory = useMessageStore(
    (s) => s.bySession.get(sessionId)?.loadingHistory ?? false,
  );
  const fromSeq = useMessageStore(
    (s) => s.bySession.get(sessionId)?.fromSeq ?? 0,
  );
  const userDetailBefore = useMessageStore(
    (s) => s.bySession.get(sessionId)?.userDetailBefore ?? 0,
  );
  const hydrated = useMessageStore(
    (s) => s.bySession.get(sessionId)?.hydrated ?? false,
  );
  const runState = useTurnStore(
    (s) => s.byId.get(sessionId)?.runState ?? "idle",
  );
  const loadMoreHistoryAction = useMessageStore((s) => s.loadMoreHistory);
  const loadMoreHistory = useCallback(() => {
    loadMoreHistoryAction(sessionId);
  }, [loadMoreHistoryAction, sessionId]);

  const listRef = useRef<HTMLDivElement>(null);
  const revealSeqRef = useRef<((seq: number) => void) | null>(null);
  const [blurOpacity, setBlurOpacity] = useState(0);
  const pendingReveal = useSyncExternalStore(subscribePendingReveal, getPendingReveal);

  // Same reveal contract as the writable shell (Search hit → load the seq, then
  // scroll it into view) so a read-only panel can locate a search result.
  useEffect(() => {
    if (!pendingReveal || pendingReveal.sessionId !== sessionId) return;
    if (!hydrated) return;
    const gen = pendingReveal.gen;
    const seq = pendingReveal.seq;
    let cancelled = false;
    void (async () => {
      const ok = await useMessageStore.getState().ensureSeqLoaded(
        sessionId,
        seq,
        () => !cancelled && getPendingReveal()?.gen === gen,
      );
      if (cancelled) return;
      if (!ok) {
        clearPendingReveal(gen);
        return;
      }
      revealSeqRef.current?.(seq);
      clearPendingReveal(gen);
    })();
    return () => {
      cancelled = true;
    };
  }, [sessionId, hydrated, pendingReveal?.sessionId, pendingReveal?.seq, pendingReveal?.gen]);

  const canLoadMore = fromSeq > 0;
  const isRunning = runState === "running" || runState === "cancelling";

  const onScroll = () => {
    const el = listRef.current;
    if (!el) return;
    setBlurOpacity(Math.min(el.scrollTop / 72, 1));
  };

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
              userDetailBefore={userDetailBefore}
              isRunning={isRunning}
              scrollRef={listRef}
              sessionId={sessionId}
              readOnly
              revealSeqRef={revealSeqRef}
            />
          </div>
        </div>
      </div>
      <div
        aria-hidden
        className={`pointer-events-none absolute inset-0 transition-opacity duration-200 ease-out ${
          isActive ? "opacity-0" : "opacity-[0.33]"
        }`}
        style={{ background: "var(--_dk-editor)" }}
      />
      <ProgressiveBlur
        side="top"
        opacity={blurOpacity}
        tintColor="var(--_dk-editor)"
        tint={1}
        height={56}
        strength={5}
        tintCurve={1}
        offset={16}
      />
    </div>
  );
}

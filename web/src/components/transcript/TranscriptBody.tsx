import {
  useCallback,
  useEffect,
  useState,
  useSyncExternalStore,
  type RefObject,
} from "react";

import {
  clearPendingReveal,
  getPendingReveal,
  subscribePendingReveal,
} from "../../lib/sessionPanelNav";
import { displayMessages, useMessageStore } from "../../stores/messageStore";
import { useTurnStore } from "../../stores/turnStore";
import { ProgressiveBlur } from "../ProgressiveBlur";
import type { RevealSeq } from "./transcriptScrollGlide";

/*
 * Transcript pieces shared by the writable shell (`AgentPanel`) and the
 * read-only child view (`SubagentReadOnlyContent`). Only the feed, the reveal
 * contract and the chrome are shared; each side keeps its own DOM layout
 * (the writable one adds the user rail and edit/mini-chat wiring).
 */

/** Message projection + paging state that feeds `MessageList`. */
export function useTranscriptFeed(sessionId: string) {
  const messages = useMessageStore((s) =>
    displayMessages(s.bySession.get(sessionId)),
  );
  const loadingHistory = useMessageStore(
    (s) => s.bySession.get(sessionId)?.loadingHistory ?? false,
  );
  const fromSeq = useMessageStore(
    (s) => s.bySession.get(sessionId)?.fromSeq ?? 0,
  );
  const runState = useTurnStore(
    (s) => s.byId.get(sessionId)?.runState ?? "idle",
  );
  const loadMoreHistoryAction = useMessageStore((s) => s.loadMoreHistory);
  const loadMoreHistory = useCallback(() => {
    loadMoreHistoryAction(sessionId);
  }, [loadMoreHistoryAction, sessionId]);
  return {
    messages,
    loadingHistory,
    canLoadMore: fromSeq > 0,
    isRunning: runState === "running" || runState === "cancelling",
    loadMoreHistory,
  };
}

/** Search hit → load the pending seq once hydrated, then scroll it into view. */
export function usePendingRevealScroll(
  sessionId: string,
  revealSeqRef: RefObject<RevealSeq | null>,
): void {
  const pendingReveal = useSyncExternalStore(
    subscribePendingReveal,
    getPendingReveal,
  );
  const hydrated = useMessageStore(
    (s) => s.bySession.get(sessionId)?.hydrated ?? false,
  );

  useEffect(() => {
    if (!pendingReveal || pendingReveal.sessionId !== sessionId) return;
    if (!hydrated) return;
    const gen = pendingReveal.gen;
    const seq = pendingReveal.seq;
    let cancelled = false;
    void (async () => {
      const ok = await useMessageStore
        .getState()
        .ensureSeqLoaded(
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
  }, [
    sessionId,
    hydrated,
    pendingReveal?.sessionId,
    pendingReveal?.seq,
    pendingReveal?.gen,
  ]);
}

/** Top blur strength driven by the scroller's offset (0 → 1 over 72px). */
export function useTopBlur(listRef: RefObject<HTMLDivElement | null>) {
  const [blurOpacity, setBlurOpacity] = useState(0);
  const onScroll = () => {
    const el = listRef.current;
    if (!el) return;
    setBlurOpacity(Math.min(el.scrollTop / 72, 1));
  };
  return { blurOpacity, onScroll };
}

/** Unfocused-panel dimming veil over the transcript. */
export function TranscriptVeil({ isActive }: { isActive: boolean }) {
  return (
    <div
      aria-hidden
      className={`pointer-events-none absolute inset-0 transition-opacity duration-200 ease-out ${
        isActive ? "opacity-0" : "opacity-[0.33]"
      }`}
      style={{ background: "var(--_dk-editor)" }}
    />
  );
}

/** Blur band under the panel's top edge. */
export function TranscriptTopBlur({ opacity }: { opacity: number }) {
  return (
    <ProgressiveBlur
      side="top"
      opacity={opacity}
      tintColor="var(--_dk-editor)"
      tint={1}
      height={56}
      strength={5}
      tintCurve={1}
      offset={16}
    />
  );
}

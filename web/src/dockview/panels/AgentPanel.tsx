import { Component, type ReactNode, useCallback, useEffect, useRef, useState } from "react";
import type { IDockviewPanelProps } from "dockview-react";

import { useConnectionStore } from "../../stores/connectionStore";
import { useSessionStore } from "../../stores/sessionStore";
import { useToastStore } from "../../stores/toastStore";
import { useTurnStore } from "../../stores/turnStore";
import { useMessageStore } from "../../stores/messageStore";
import { useNotificationStore } from "../../stores/notificationStore";
import { AgentChatInput } from "../../components/AgentChatInput";
import { MessageList } from "../../components/MessageList";
import { clearFoldCardOpen } from "../../components/foldCardState";
import { ProgressiveBlur } from "../../components/ProgressiveBlur";
import { TodoPanel } from "../../components/TodoPanel";

class AgentErrorBoundary extends Component<
  { onClose: () => void; children: ReactNode },
  { hasError: boolean }
> {
  state = { hasError: false };
  static getDerivedStateFromError() {
    return { hasError: true };
  }
  render() {
    if (this.state.hasError) {
      return <PanelCrash onClose={this.props.onClose} />;
    }
    return this.props.children;
  }
}

function PanelCrash({ onClose }: { onClose: () => void }) {
  return (
    <div className="flex h-full flex-col items-center justify-center gap-4 px-4 text-(--_dk-text-muted)">
      <p className="text-sm">Something went wrong with this session.</p>
      <button
        type="button"
        onClick={onClose}
        className="btn btn-sm"
      >
        Close
      </button>
    </div>
  );
}

// Self-contained per-session chat view (dockview center-grid tab).
// This panel OWNS its subscription lifecycle: it subscribes whenever the
// socket becomes usable (first connect or every reconnect) and closes
// itself if the session no longer exists. The connection store clears
// `subscribedSessions` on every drop, so re-calling ensureSubscribe here
// always re-arms the server-side subscription after a reconnect.
export function AgentPanel(props: IDockviewPanelProps) {
  const sessionId = (props.params as { sessionId?: string }).sessionId ?? "";
  const connState = useConnectionStore((s) => s.state);

  // (Re)subscribe while the socket is usable. Re-runs on every transition to
  // "connected", including reconnects, so a dropped subscription self-heals.
  useEffect(() => {
    if (!sessionId || connState !== "connected") return;
    let disposed = false;
    useConnectionStore.getState().ensureSubscribe(sessionId).catch((error: unknown) => {
      if (disposed) return;
      const message = error instanceof Error ? error.message : "Failed to open session";
      if (/session.*not found/i.test(message)) {
        useToastStore.getState().showToast("This session no longer exists", "error");
        props.api.close();
      }
      // Any other failure (socket dropped mid-flight, timeout) is left to the
      // next "connected" transition rather than surfaced as a scary toast.
    });
    return () => {
      disposed = true;
    };
  }, [props.api, sessionId, connState]);

  // Tear down the subscription and local projection only on real unmount.
  useEffect(() => {
    if (!sessionId) return;
    return () => {
      useConnectionStore.getState().unsubscribeSession(sessionId);
      useMessageStore.getState().reset(sessionId);
      useTurnStore.getState().resetTurn(sessionId);
      useNotificationStore.getState().reset(sessionId);
      clearFoldCardOpen(sessionId);
    };
  }, [sessionId]);

  // Mirror the session-list preview as the tab title. The default dockview
  // tab clamps/truncates the text, so we get the same "summary" form the
  // SessionItem shows. Falls back to a short id until the list has loaded
  // (and thus populated the preview) for this session.
  const preview = useSessionStore(
    (s) => s.sessions.find((x) => x.id === sessionId)?.preview?.trim() ?? "",
  );
  useEffect(() => {
    props.api.setTitle(preview || sessionId.slice(0, 8));
  }, [props.api, sessionId, preview]);

  const close = () => {
    props.api.close();
  };

  if (!sessionId) {
    return <PanelCrash onClose={close} />;
  }

  return (
    <AgentErrorBoundary onClose={close}>
      <AgentChatShell sessionId={sessionId} />
    </AgentErrorBoundary>
  );
}

/** Layout shell — no turn/message business subscriptions.
 * Exported for Remotion compositions (c_msg_stream) that render the real
 * chat shell against a mocked store — the dockview panel entry above is
 * not usable headless. */
export function AgentChatShell({ sessionId }: { sessionId: string }) {
  return (
    <div className="relative flex h-full flex-col">
      <MessageListRegion sessionId={sessionId} />
      <ComposerDock sessionId={sessionId} />
    </div>
  );
}

/**
 * Transcript region — the only parent that feeds MessageList.
 * Subscribes to message fields + runState boolean; never to composer draft.
 */
function MessageListRegion({ sessionId }: { sessionId: string }) {
  const messages = useMessageStore(
    (s) => s.bySession.get(sessionId)?.messages ?? EMPTY_MESSAGES,
  );
  const loadingHistory = useMessageStore(
    (s) => s.bySession.get(sessionId)?.loadingHistory ?? false,
  );
  const bufferViewStart = useMessageStore(
    (s) => s.bySession.get(sessionId)?.bufferViewStart ?? 0,
  );
  const userDetailBefore = useMessageStore(
    (s) => s.bySession.get(sessionId)?.userDetailBefore ?? 0,
  );
  const shapeError = useMessageStore(
    (s) => s.bySession.get(sessionId)?.shapeError ?? null,
  );
  const turnEndNotice = useMessageStore(
    (s) => s.bySession.get(sessionId)?.turnEndNotice ?? null,
  );
  const runState = useTurnStore(
    (s) => s.byId.get(sessionId)?.runState ?? "idle",
  );
  const loadMoreHistoryAction = useMessageStore((s) => s.loadMoreHistory);
  const loadMoreHistory = useCallback(() => {
    loadMoreHistoryAction(sessionId);
  }, [loadMoreHistoryAction, sessionId]);

  const listRef = useRef<HTMLDivElement>(null);
  const [blurOpacity, setBlurOpacity] = useState(0);

  const canLoadMore = bufferViewStart > 0;
  const isRunning = runState === "running" || runState === "cancelling";
  const maxFileRevertK = useSessionStore(
    (s) => s.byId.get(sessionId)?.maxFileRevertK ?? null,
  );

  const onScroll = () => {
    const el = listRef.current;
    if (!el) return;
    setBlurOpacity(Math.min(el.scrollTop / 72, 1));
    if (el.scrollTop < 32 && canLoadMore && !loadingHistory && !isRunning) {
      loadMoreHistory();
    }
  };

  return (
    <>
      {shapeError && (
        <div className="shrink-0 border-b border-(--_dk-red-500)/30 bg-(--_dk-red-500)/10 px-3 py-2 text-xs text-(--_dk-red-500)">
          {shapeError}
        </div>
      )}
      {turnEndNotice && (
        <div className="shrink-0 border-b border-(--_dk-red-500)/30 bg-(--_dk-red-500)/10 px-3 py-2 text-xs text-(--_dk-red-500)">
          {turnEndNotice.message}
        </div>
      )}

      {/* Three layers, each with one job:
          1. Non-scrolling frame (this div): carries the PERSISTENT top/side
             inset (pt-4/px-4). Because it never scrolls, the scroll container
             inside is permanently pushed 16px from the panel edges — content
             can never touch the top/side while scrolling. NOT `relative` so the
             blur band below stays anchored to AgentChatShell.
          2. Scroll container (middle): full-width within the frame, so its
             scrollbar rides the panel's right edge (not centered with content).
             This is the element the virtualizer measures (ref/listRef).
          3. Content column (inner): centered reading measure only (mx-auto
             max-w) — no padding here, the frame already provides the inset. */}
      <div className="flex min-h-0 flex-1 flex-col bg-(--_dk-editor) px-4 pt-4">
        <div
          ref={listRef}
          onScroll={onScroll}
          className="min-h-0 flex-1 overflow-y-auto bg-(--_dk-editor) [container-type:size]"
        >
          {/*
            Size to content, not to the viewport. `flex-1 min-h-0` on this
            column made the virtualizer's scrollHeight fight the flex box
            (viewport-sized child + overflowing absolute items), which is one
            of the "list drifts while streaming" sources.
          */}
          <div
            className={`mx-auto flex w-full max-w-[var(--_dk-prose-measure)] flex-col bg-(--_dk-editor) ${
              messages.length === 0 ? "min-h-full" : ""
            }`}
          >
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
              maxFileRevertK={maxFileRevertK}
            />
          </div>
          {/* Half the scrollport. Direct child so `h-1/2` is 50% of the
              list viewport, not of content. Pushes the last bubble up;
              native stick-to-end follows scrollHeight, which includes this. */}
          {messages.length > 0 && (
            <div aria-hidden className="h-[50cqh] w-full shrink-0" />
          )}
        </div>
      </div>
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
    </>
  );
}

const EMPTY_MESSAGES: import("../../api/adapter").ChatRow[] = [];

/** Composer + todos — no messageStore subscription.
 *  Floats over the transcript at the same reading measure as MessageList,
 *  so the list can scroll under it instead of being clipped above. */
function ComposerDock({ sessionId }: { sessionId: string }) {
  return (
    <div className="pointer-events-none absolute inset-x-0 bottom-0 z-10 px-4 pb-3">
      <div className="pointer-events-auto mx-auto flex w-full max-w-[var(--_dk-prose-measure)] flex-col gap-2">
        <TodoPanel sessionId={sessionId} />
        <AgentChatInput key={sessionId} sessionId={sessionId} />
      </div>
    </div>
  );
}

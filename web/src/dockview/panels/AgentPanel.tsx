import { Component, type ReactNode, type RefObject, useCallback, useEffect, useRef, useState } from "react";
import type { IDockviewPanelProps } from "dockview-react";
import { CaretDownIcon } from "@phosphor-icons/react";

import { useConnectionStore } from "../../stores/connectionStore";
import { useSessionStore } from "../../stores/sessionStore";
import { useToastStore } from "../../stores/toastStore";
import { useTurnStore } from "../../stores/turnStore";
import { displayMessages, useMessageStore } from "../../stores/messageStore";
import { useNotificationStore } from "../../stores/notificationStore";
import { AgentChatInput } from "../../components/AgentChatInput";
import { MessageList, type EditingUserAnchor } from "../../components/MessageList";
import { PermissionCard } from "../../components/PermissionModal";
import { clearFoldCardOpen } from "../../components/foldCardState";
import { ProgressiveBlur } from "../../components/ProgressiveBlur";
import { TodoPanel } from "../../components/TodoPanel";
import { TerminalStatusBar } from "../../components/TerminalStatusBar";
import { composerCardClass } from "../../components/composerCard";

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
  const [isActive, setIsActive] = useState(props.api.isActive);

  // Track the dockview panel active state — drives the focused/unfocused
  // emphasis (bigger + brighter vs smaller + dimmer) on the whole chat shell.
  useEffect(() => {
    const d = props.api.onDidActiveChange((e) => setIsActive(e.isActive));
    return () => d.dispose();
  }, [props.api]);

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
      <AgentChatShell sessionId={sessionId} isActive={isActive} />
    </AgentErrorBoundary>
  );
}

/** Layout shell — no turn/message business subscriptions.
 * Exported for Remotion compositions (c_msg_stream) that render the real
 * chat shell against a mocked store — the dockview panel entry above is
 * not usable headless. */
export function AgentChatShell({
  sessionId,
  isActive = true,
}: {
  sessionId: string;
  isActive?: boolean;
}) {
  const [stickToEnd, setStickToEnd] = useState(true);
  const [editingAnchor, setEditingAnchor] = useState<EditingUserAnchor | null>(null);
  const [miniPhase, setMiniPhase] = useState<"idle" | "entering" | "visible" | "exiting">("idle");
  const dismissTimerRef = useRef<number | null>(null);
  const jumpToEndRef = useRef<(() => void) | null>(null);
  const revealBashRef = useRef<((callId: string) => void) | null>(null);

  const openMini = useCallback((anchor: EditingUserAnchor) => {
    if (dismissTimerRef.current !== null) {
      clearTimeout(dismissTimerRef.current);
      dismissTimerRef.current = null;
    }
    setEditingAnchor(anchor);
    setMiniPhase("entering");
  }, []);

  const finishDismiss = useCallback(() => {
    if (dismissTimerRef.current !== null) {
      clearTimeout(dismissTimerRef.current);
      dismissTimerRef.current = null;
    }
    setMiniPhase("idle");
    setEditingAnchor(null);
  }, []);

  const dismiss = useCallback(() => {
    if (miniPhase === "visible" || miniPhase === "entering") {
      setMiniPhase("exiting");
      dismissTimerRef.current = window.setTimeout(finishDismiss, 180);
    }
  }, [finishDismiss, miniPhase]);

  useEffect(
    () => () => {
      if (dismissTimerRef.current !== null) {
        clearTimeout(dismissTimerRef.current);
      }
    },
    [],
  );

  useEffect(() => {
    if (!editingAnchor) return;
    const dismissOutside = (event: MouseEvent) => {
      const target = event.target;
      if (
        target instanceof Element &&
        target.closest(
          "[data-mini-chat-input], [data-user-message-bubble], [data-dropdown-panel]",
        )
      ) {
        return;
      }
      dismiss();
    };
    document.addEventListener("mousedown", dismissOutside);
    return () => document.removeEventListener("mousedown", dismissOutside);
  }, [editingAnchor, dismiss]);

  return (
    <div className="relative flex h-full flex-col">
      <MessageListRegion
        sessionId={sessionId}
        isActive={isActive}
        editingAnchor={editingAnchor}
        onEditAnchor={openMini}
        onDismissEdit={dismiss}
        miniPhase={miniPhase}
        onMiniAnimationEnd={() => {
          if (miniPhase === "entering") setMiniPhase("visible");
          if (miniPhase === "exiting") finishDismiss();
        }}
        onStickChange={setStickToEnd}
        jumpToEndRef={jumpToEndRef}
        revealBashRef={revealBashRef}
      />
      <ComposerDock
        sessionId={sessionId}
        isActive={isActive}
        stickToEnd={stickToEnd}
        onJumpToEnd={() => jumpToEndRef.current?.()}
        onRevealBash={(callId) => revealBashRef.current?.(callId)}
      />
    </div>
  );
}

/**
 * Transcript region — the only parent that feeds MessageList.
 * Subscribes to message fields + runState boolean; never to composer draft.
 */
function MessageListRegion({
  sessionId,
  isActive,
  editingAnchor,
  onEditAnchor,
  onDismissEdit,
  miniPhase,
  onMiniAnimationEnd,
  onStickChange,
  jumpToEndRef,
  revealBashRef,
}: {
  sessionId: string;
  isActive: boolean;
  editingAnchor: EditingUserAnchor | null;
  onEditAnchor: (anchor: EditingUserAnchor) => void;
  onDismissEdit: () => void;
  miniPhase: "idle" | "entering" | "visible" | "exiting";
  onMiniAnimationEnd: () => void;
  onStickChange: (stickToEnd: boolean) => void;
  jumpToEndRef: RefObject<(() => void) | null>;
  revealBashRef: RefObject<((callId: string) => void) | null>;
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
  const runState = useTurnStore(
    (s) => s.byId.get(sessionId)?.runState ?? "idle",
  );
  const loadMoreHistoryAction = useMessageStore((s) => s.loadMoreHistory);
  const loadMoreHistory = useCallback(() => {
    loadMoreHistoryAction(sessionId);
  }, [loadMoreHistoryAction, sessionId]);

  const listRef = useRef<HTMLDivElement>(null);
  const [blurOpacity, setBlurOpacity] = useState(0);

  const canLoadMore = fromSeq > 0;
  const isRunning = runState === "running" || runState === "cancelling";
  const maxFileRevertK = useSessionStore(
    (s) => s.byId.get(sessionId)?.maxFileRevertK ?? null,
  );

  const onScroll = () => {
    const el = listRef.current;
    if (!el) return;
    setBlurOpacity(Math.min(el.scrollTop / 72, 1));
  };

  return (
    <>
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
      <div className="relative flex min-h-0 flex-1 flex-col">
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
                maxFileRevertK={maxFileRevertK}
                onStickChange={onStickChange}
                jumpToEndRef={jumpToEndRef}
                revealBashRef={revealBashRef}
                editingAnchor={editingAnchor}
                onEditAnchor={onEditAnchor}
                onDismissEdit={onDismissEdit}
                miniPhase={miniPhase}
                onMiniAnimationEnd={onMiniAnimationEnd}
              />
            </div>
          </div>
        </div>
        {/* Unfocused dimming — pure visual mask, never touches content alpha.
            Instead of fading the list's own opacity (which re-composites every
            message and can break nested backdrop-filter), a translucent
            panel-color veil is painted on top. pointer-events-none so it never
            blocks or intercepts any interaction (scroll, click, drag, hover). */}
        <div
          aria-hidden
          className={`pointer-events-none absolute inset-0 transition-opacity duration-200 ease-out ${
            isActive ? "opacity-0" : "opacity-[0.33]"
          }`}
          style={{ background: "var(--_dk-editor)" }}
        />
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

/** Composer + todos + permission — no messageStore subscription.
 *  Floats over the transcript at the same reading measure as MessageList,
 *  so the list can scroll under it instead of being clipped above. */
export function ComposerDock({
  sessionId,
  isActive = true,
  stickToEnd = true,
  onJumpToEnd,
  onRevealBash,
}: {
  sessionId: string;
  isActive?: boolean;
  stickToEnd?: boolean;
  onJumpToEnd?: () => void;
  onRevealBash?: (callId: string) => void;
}) {
  const pendingPermission = useTurnStore(
    (s) => s.byId.get(sessionId)?.pendingPermission ?? null,
  );
  const grantPermission = useTurnStore((s) => s.grantPermission);

  return (
    <div className="pointer-events-none absolute inset-x-0 bottom-0 z-10 px-4 pb-4">
      <div
        className={`pointer-events-auto mx-auto flex w-full max-w-[var(--_dk-prose-measure)] flex-col gap-2 ${
          isActive
            ? "[--_dk-composer-card-shadow:var(--_dk-composer-focus-shadow)]"
            : ""
        }`}
      >
        {!stickToEnd && (
          <div className="flex justify-center">
            <button
              type="button"
              className={`${composerCardClass} inline-flex items-center gap-1 px-2.5 py-1 text-xs text-(--_dk-text-secondary) transition-transform duration-100 hover:scale-105 active:scale-90 active:brightness-90`}
              onClick={onJumpToEnd}
            >
              <CaretDownIcon size={12} weight="bold" aria-hidden />
              Latest
            </button>
          </div>
        )}
        {pendingPermission && (
          <PermissionCard
            tool={pendingPermission.tool}
            ruleId={pendingPermission.rule_id}
            summary={pendingPermission.summary}
            requestId={pendingPermission.request_id}
            onGrant={(approved, always) => {
              grantPermission(sessionId, approved, always);
            }}
          />
        )}
        <div className="flex items-end gap-2">
          <TerminalStatusBar sessionId={sessionId} onRevealBash={onRevealBash} />
          <div className="min-w-0 flex-1">
            <TodoPanel sessionId={sessionId} />
          </div>
        </div>
        <AgentChatInput key={sessionId} sessionId={sessionId} />
      </div>
    </div>
  );
}

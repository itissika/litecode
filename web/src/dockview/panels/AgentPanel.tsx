import {
  Component,
  type ReactNode,
  type RefObject,
  useCallback,
  useEffect,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";
import type { IDockviewPanelProps } from "dockview-react";
import { CaretDownIcon } from "@phosphor-icons/react";

import { useConnectionStore } from "../../stores/connectionStore";
import { useSessionStore } from "../../stores/sessionStore";
import { useToastStore } from "../../stores/toastStore";
import { useTurnStore } from "../../stores/turnStore";
import { displayMessages, useMessageStore } from "../../stores/messageStore";
import {
  clearPendingReveal,
  fallbackSessionTitle,
  getPendingReveal,
  subscribePendingReveal,
} from "../../lib/sessionPanelNav";
import { AgentChatInput } from "../../components/AgentChatInput";
import {
  MessageList,
  type EditingUserAnchor,
} from "../../components/MessageList";
import { LlmReconnectBubble } from "../../components/LlmReconnectBubble";
import { PermissionCard } from "../../components/PermissionModal";
import { ProgressiveBlur } from "../../components/ProgressiveBlur";
import { SessionStatusLine } from "../../components/SessionStatusLine";
import { releaseSessionTab } from "../../components/sessionTeardown";
import { hostElementFromTarget, viewOf } from "../../lib/domView";
import { SubagentReadOnlyContent } from "../../components/SubagentReadOnlyContent";
import { composerCardClass } from "../../components/composerCard";
import { UserMessageRail } from "../../components/transcript/UserMessageRail";
import { useScrollUserAnchors } from "../../components/transcript/useScrollUserAnchors";
import type { RevealSeq } from "../../components/transcript/transcriptScrollGlide";
import {
  USER_RAIL_PAD_LEFT,
  USER_RAIL_PAD_RIGHT,
  USER_RAIL_WIDTH_MAX,
  type UserRailLayoutMark,
} from "../../components/transcript/transcriptUserRailMarks";

/** Rail strip at its widest: 12px outer padding + 12px ticks + 8px gap. */
const RAIL_BOX_MAX =
  USER_RAIL_WIDTH_MAX + USER_RAIL_PAD_LEFT + USER_RAIL_PAD_RIGHT;
/** Message column insets, tuned alongside the rail strip. */
const LIST_PAD_LEFT = 4;
const LIST_PAD_RIGHT = 4;

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
      <button type="button" onClick={onClose} className="btn btn-sm">
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
  const params = props.params as { sessionId?: string; sessionKind?: string };
  const sessionId = params.sessionId ?? "";
  // TRUSTED ENTRY PROVENANCE. Only `openSessionPanel` — the writable, root-only
  // navigation entry (SessionList filtered to roots, `newSession`, and a
  // root-confirmed Search hit) — tags its params with `sessionKind: "root"`. A
  // freshly created root is not in `session/list` yet, so without this it would
  // fail closed to a blank read-only transcript. We deliberately do NOT infer
  // writability from the `agent-*` panel id (that reopens the child bypass) and
  // do NOT guess with a timeout: absent metadata still fails closed. A restored
  // legacy `agent-<child>` panel carries no such param and stays read-only.
  const explicitRootIntent = params.sessionKind === "root";
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
    useConnectionStore
      .getState()
      .ensureSubscribe(sessionId)
      .catch((error: unknown) => {
        if (disposed) return;
        const message =
          error instanceof Error ? error.message : "Failed to open session";
        if (/session.*not found/i.test(message)) {
          useToastStore
            .getState()
            .showToast("This session no longer exists", "error");
          props.api.close();
        }
        // Any other failure (socket dropped mid-flight, timeout) is left to the
        // next "connected" transition rather than surfaced as a scary toast.
      });
    return () => {
      disposed = true;
    };
  }, [props.api, sessionId, connState]);

  // Tear down the subscription and local projection only on real unmount — and
  // not at all while an expanded roster card still holds the session.
  useEffect(() => {
    if (!sessionId) return;
    return () => {
      releaseSessionTab(sessionId);
    };
  }, [sessionId]);

  // Mirror the session-list preview as the tab title. The default dockview
  // tab clamps/truncates the text, so we get the same "summary" form the
  // SessionItem shows. Falls back to "NEW" for a writable session that has no
  // preview yet (a freshly created one); a child keeps its short id.
  const preview = useSessionStore(
    (s) => s.sessions.find((x) => x.id === sessionId)?.preview?.trim() ?? "",
  );

  // Identity classification against the known session list. This is the SAFETY
  // boundary: a legacy/restored `agent-*` panel can carry a CHILD session id
  // (old layout, or a Search hit opened before the read-only phase), and a
  // Search hit can land here for a session the list has not classified yet.
  //
  // The HIGHEST invariant is "a known child is always read-only". So metadata
  // WINS over params: once the list confirms `parent_session_id`, the panel is
  // read-only no matter what provenance the params claim (a stale/forged layout
  // param must never reopen a child). Only when the session is NOT a known child
  // do we trust either the trusted root intent (fresh `newSession` root, absent
  // from the list) or a present session (known root). An unknown id with no
  // intent fails closed.
  const session = useSessionStore((s) =>
    s.sessions.find((x) => x.id === sessionId),
  );
  const knownChild = session !== undefined && !!session.parent_session_id;
  const writable = !knownChild && (explicitRootIntent || session !== undefined);
  useEffect(() => {
    props.api.setTitle(preview || fallbackSessionTitle(sessionId, writable));
  }, [props.api, sessionId, preview, writable]);

  const close = () => {
    props.api.close();
  };

  if (!sessionId) {
    return <PanelCrash onClose={close} />;
  }

  return (
    <AgentErrorBoundary onClose={close}>
      {writable ? (
        <AgentChatShell sessionId={sessionId} isActive={isActive} />
      ) : (
        <SubagentReadOnlyContent sessionId={sessionId} isActive={isActive} />
      )}
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
  const [editingAnchor, setEditingAnchor] = useState<EditingUserAnchor | null>(
    null,
  );
  const [miniPhase, setMiniPhase] = useState<
    "idle" | "entering" | "visible" | "exiting"
  >("idle");
  // Owned by the dock, published up so the transcript can shrink its bottom pad
  // (the pad only exists to keep the tail clear of the floating composer).
  const [composerCollapsed, setComposerCollapsed] = useState(false);
  const dismissTimerRef = useRef<number | null>(null);
  const dismissTimerViewRef = useRef<Window | null>(null);
  const shellRef = useRef<HTMLDivElement>(null);
  const jumpToEndRef = useRef<(() => void) | null>(null);
  const revealBashRef = useRef<((callId: string) => void) | null>(null);
  const revealSeqRef = useRef<RevealSeq | null>(null);
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

  const openMini = useCallback((anchor: EditingUserAnchor) => {
    if (dismissTimerRef.current !== null) {
      clearTimeout(dismissTimerRef.current);
      dismissTimerRef.current = null;
    }
    setEditingAnchor(anchor);
    // Bubble click / re-open: start the enter animation. Draft edits from the
    // mini chat itself must NOT restart it (the wrapper would collapse back
    // to the bubble height on every keystroke).
    setMiniPhase((phase) =>
      phase === "idle" || phase === "exiting" ? "entering" : phase,
    );
  }, []);

  const clearDismissTimer = useCallback(() => {
    if (dismissTimerRef.current === null) return;
    (dismissTimerViewRef.current ?? window).clearTimeout(dismissTimerRef.current);
    dismissTimerRef.current = null;
    dismissTimerViewRef.current = null;
  }, []);

  const finishDismiss = useCallback(() => {
    clearDismissTimer();
    setMiniPhase("idle");
    setEditingAnchor(null);
  }, [clearDismissTimer]);

  const dismiss = useCallback(() => {
    if (miniPhase === "visible" || miniPhase === "entering") {
      setMiniPhase("exiting");
      clearDismissTimer();
      const view = viewOf(shellRef.current);
      dismissTimerViewRef.current = view;
      dismissTimerRef.current = view.setTimeout(finishDismiss, 180);
    }
  }, [clearDismissTimer, finishDismiss, miniPhase]);

  useEffect(() => () => clearDismissTimer(), [clearDismissTimer]);

  useEffect(() => {
    if (!editingAnchor) return;
    const root = shellRef.current;
    const doc = root?.ownerDocument ?? document;
    const dismissOutside = (event: MouseEvent) => {
      const target = hostElementFromTarget(event.target, root);
      if (
        target?.closest(
          "[data-mini-chat-input], [data-user-message-bubble], [data-dropdown-panel]",
        )
      ) {
        return;
      }
      dismiss();
    };
    doc.addEventListener("mousedown", dismissOutside);
    return () => doc.removeEventListener("mousedown", dismissOutside);
  }, [editingAnchor, dismiss]);

  return (
    <div ref={shellRef} className="relative flex h-full flex-col">
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
        revealSeqRef={revealSeqRef}
        composerCollapsed={composerCollapsed}
      />
      <ComposerDock
        sessionId={sessionId}
        isActive={isActive}
        stickToEnd={stickToEnd}
        onJumpToEnd={() => jumpToEndRef.current?.()}
        onRevealBash={(callId) => revealBashRef.current?.(callId)}
        onCollapsedChange={setComposerCollapsed}
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
  revealSeqRef,
  composerCollapsed,
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
  revealSeqRef: RefObject<RevealSeq | null>;
  composerCollapsed: boolean;
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
  const runState = useTurnStore(
    (s) => s.byId.get(sessionId)?.runState ?? "idle",
  );
  const loadMoreHistoryAction = useMessageStore((s) => s.loadMoreHistory);
  const loadMoreHistory = useCallback(() => {
    loadMoreHistoryAction(sessionId);
  }, [loadMoreHistoryAction, sessionId]);

  const listRef = useRef<HTMLDivElement>(null);
  const userRailLayoutRef = useRef<UserRailLayoutMark[]>([]);
  const userRailNotifyRef = useRef<(() => void) | null>(null);
  const userRail = useScrollUserAnchors(sessionId);
  const [blurOpacity, setBlurOpacity] = useState(0);

  const canLoadMore = fromSeq > 0;
  const isRunning = runState === "running" || runState === "cancelling";
  const maxFileRevertSeq = useSessionStore(
    (s) => s.byId.get(sessionId)?.maxFileRevertSeq ?? null,
  );

  const onScroll = () => {
    const el = listRef.current;
    if (!el) return;
    setBlurOpacity(Math.min(el.scrollTop / 72, 1));
  };

  return (
    <>
      {/* Three layers, each with one job:
          1. Non-scrolling frame: keeps the transcript 16px from the panel's
             top only. No side padding: the scroll viewport spans the panel and
             the content margins come from the columns themselves.
             NOT `relative` so the blur band below stays anchored to AgentChatShell.
          2. Scroll container (middle): full-width; its 8px scrollbar rides the
             panel's right edge and is not counted in the content margins. This
             is the element the virtualizer measures.
          3. Content row (inner): the rail strip beside the message list. The
             strip owns a 12px outer inset and an 8px gap to the messages, the
             list pads 4px on both sides, so the tick-to-prose gap is 12px and
             the right margin (4px + the 8px scrollbar) mirrors it. At max
             width the strip box is 32px and the list keeps its 72ch measure. */}
      <div className="relative flex min-h-0 flex-1 flex-col">
        <div className="flex min-h-0 flex-1 flex-col bg-(--_dk-editor) pt-4">
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
              className="mx-auto flex w-full bg-(--_dk-editor)"
              style={{
                maxWidth: `calc(var(--_dk-prose-measure) + ${RAIL_BOX_MAX + LIST_PAD_LEFT + LIST_PAD_RIGHT}px)`,
              }}
            >
              <UserMessageRail
                sessionId={sessionId}
                scrollRef={listRef}
                layoutRef={userRailLayoutRef}
                notifyRef={userRailNotifyRef}
                canLoadMore={canLoadMore}
                onCenterSeq={userRail.noteCenter}
                onJump={(seq) => {
                  onStickChange(false);
                  void useMessageStore
                    .getState()
                    .ensureSeqLoaded(sessionId, seq)
                    .then((ok) => {
                      if (ok) revealSeqRef.current?.(seq, { glide: true });
                    });
                }}
              />
              <div
                className="relative min-w-0 flex-1"
                style={{ paddingLeft: LIST_PAD_LEFT, paddingRight: LIST_PAD_RIGHT }}
              >
                <MessageList
                  key={sessionId}
                  messages={messages}
                  loadingHistory={loadingHistory}
                  canLoadMore={canLoadMore}
                  onLoadMore={loadMoreHistory}
                  isRunning={isRunning}
                  scrollRef={listRef}
                  sessionId={sessionId}
                  maxFileRevertSeq={maxFileRevertSeq}
                  onStickChange={onStickChange}
                  jumpToEndRef={jumpToEndRef}
                  revealBashRef={revealBashRef}
                  revealSeqRef={revealSeqRef}
                  editingAnchor={editingAnchor}
                  onEditAnchor={onEditAnchor}
                  onDismissEdit={onDismissEdit}
                  miniPhase={miniPhase}
                  onMiniAnimationEnd={onMiniAnimationEnd}
                  composerCollapsed={composerCollapsed}
                  userRailLayoutRef={userRailLayoutRef}
                  userRailNotifyRef={userRailNotifyRef}
                />
              </div>
            </div>
          </div>
        </div>
        {/* Unfocused dimming covers the whole panel viewport -- transcript,
            rail ticks and the top blur band included. No z-index: as the last
            child of the frame it already paints above the transcript (the
            scroller's containment paints as in-flow content, the veil is a
            positioned descendant), so the only layer that stays undimmed is
            the composer dock (absolute inset-0 z-10, mounted after this
            region). pointer-events-none so it never blocks scroll, click or
            hover. */}
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
 *  so the list can scroll under it instead of being clipped above.
 *  Collapsible: the whole dock (Latest / permission / chips / input — no
 *  exceptions) slides straight down out of view via a plain icon toggle
 *  floating at the bottom center. */
export function ComposerDock({
  sessionId,
  isActive = true,
  stickToEnd = true,
  onJumpToEnd,
  onRevealBash,
  onCollapsedChange,
}: {
  sessionId: string;
  isActive?: boolean;
  stickToEnd?: boolean;
  onJumpToEnd?: () => void;
  onRevealBash?: (callId: string) => void;
  /** Notifies the shell so the transcript can shrink its bottom pad while the
   *  composer is out of the way. */
  onCollapsedChange?: (collapsed: boolean) => void;
}) {
  const pendingPermission = useTurnStore(
    (s) => s.byId.get(sessionId)?.pendingPermission ?? null,
  );
  const grantPermission = useTurnStore((s) => s.grantPermission);
  const [collapsed, setCollapsed] = useState(false);

  return (
    // The dock fills the pane (inset-0, pointer-events-none) and bottom-aligns
    // its column, so the browser — not a JS measurement — owns the height
    // clamp: the status panel below is the one shrinkable item (row, chips and
    // input are `shrink-0`), so a long plan is stopped at the free space above
    // the capsule row instead of growing out of the agent panel.
    <div
      data-testid="composer-dock"
      className="pointer-events-none absolute inset-0 z-10 flex min-h-0 flex-col justify-end px-4 pb-4"
    >
      {/* Hit-testing surface. The wrapper must also drop pointer events while
          collapsed: the collapse only *translates* the content out of view, so
          the wrapper keeps its layout box (the band the composer occupied) and
          a pointer-events-auto box there swallows the wheel — the transcript
          below it cannot scroll. The toggle re-enables its own events. */}
      <div
        className={`relative mx-auto flex min-h-0 w-full max-w-[var(--_dk-prose-measure)] flex-col ${
          collapsed ? "pointer-events-none" : "pointer-events-auto"
        } ${
          isActive
            ? "[--_dk-composer-card-shadow:var(--_dk-composer-focus-shadow)]"
            : ""
        }`}
      >
        {/* Collapsible content — slides straight down out of view on collapse.
            A pure transform (no layout reflow, no clip) keeps the cards' wide
            box shadows intact, and the panel's own overflow:hidden (dockview
            .dv-pane/.dv-groupview) clips it at the bottom edge. The translate
            is 100% of the content's own height plus 2rem — the 1rem clears the
            dock's own pb-4 padding, the extra 1rem guarantees the top edge
            lands below the panel even with subpixel rounding. Opacity fades to
            0 alongside the slide, so the content is fully gone regardless of
            the clip boundary. Its own pointer-events-none is belt-and-braces;
            the wrapper above is what actually frees the vacated area. */}
        <div
          data-testid="composer-dock-content"
          data-collapsed={collapsed}
          className={`composer-dock-slide flex min-h-0 flex-col ${
            collapsed
              ? "pointer-events-none translate-y-[calc(100%_+_2rem)] opacity-0"
              : "opacity-100"
          }`}
        >
          <div className="flex min-h-0 flex-col gap-2">
            {!stickToEnd && (
              <div className="flex shrink-0 justify-center">
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
                kind={pendingPermission.kind}
                freeText={pendingPermission.free_text}
                options={pendingPermission.options}
                multiSelect={pendingPermission.multi_select}
                questions={pendingPermission.questions}
                onGrant={(approved, always, opts) => {
                  grantPermission(sessionId, approved, always, opts);
                }}
              />
            )}
            <LlmReconnectBubble sessionId={sessionId} />
            <SessionStatusLine
              sessionId={sessionId}
              onRevealBash={onRevealBash}
            />
            <AgentChatInput key={sessionId} sessionId={sessionId} />
          </div>
        </div>

        {/* Collapse/expand toggle — floating at the bottom center. While the
            dock is expanded it sits on the chat input and stays transparent;
            once the content has slid out it gets the exact same card treatment
            (border + shadow + glass) as the composer cards so it stays legible
            alone at the bottom. Chevron flips. */}
        <button
          type="button"
          onClick={() => {
            const next = !collapsed;
            setCollapsed(next);
            onCollapsedChange?.(next);
          }}
          aria-label={collapsed ? "Expand composer" : "Collapse composer"}
          title={collapsed ? "Expand composer" : "Collapse composer"}
          className={`pointer-events-auto absolute bottom-1.5 left-1/2 z-20 flex h-5 w-9 -translate-x-1/2 cursor-pointer items-center justify-center active:brightness-90 ${
            collapsed
              ? // No `relative` here: it loses to nothing but *wins* over the
                // `absolute` above (Tailwind emits .relative after .absolute),
                // which drops the toggle back into the wrapper's flex flow and
                // grows it by the button's 20px. The wrapper is bottom-aligned,
                // so that 20px shoves the whole composer up for a frame — the
                // "bounce up before it slides down" on collapse. The button is
                // already a containing block for its own `after:` layer.
                `${composerCardClass} text-(--_dk-text-secondary) after:pointer-events-none after:absolute after:inset-0 after:rounded-md after:bg-(--_dk-ix-bg-hover) after:content-[''] after:opacity-0 after:transition-opacity after:duration-150 hover:after:opacity-100`
              : "rounded text-(--_dk-text-muted) transition-colors duration-200 hover:bg-(--_dk-ix-bg-hover) hover:text-(--_dk-text-secondary)"
          }`}
        >
          <CaretDownIcon
            size={12}
            weight="bold"
            aria-hidden
            className={`transition-transform duration-200 ${
              collapsed ? "rotate-180" : ""
            }`}
          />
        </button>
      </div>
    </div>
  );
}

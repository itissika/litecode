import { useConnectionStore } from "../stores/connectionStore";
import { clearFoldCardOpen } from "../components/foldCardState";
import { hasPanel } from "../dockview/workbench/queries";
import { onPanelRemoved } from "../dockview/workbench/events";
import { openPanel } from "../dockview/workbench/commands";

export interface PendingSeqReveal {
  sessionId: string;
  seq: number;
  gen: number;
}

let pending: PendingSeqReveal | null = null;
let gen = 0;
const listeners = new Set<() => void>();

function emit(): void {
  for (const listener of listeners) listener();
}

export function subscribePendingReveal(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function getPendingReveal(): PendingSeqReveal | null {
  return pending;
}

export function requestSeqReveal(
  sessionId: string,
  seq: number,
): PendingSeqReveal {
  gen += 1;
  pending = { sessionId, seq, gen };
  emit();
  return pending;
}

export function clearPendingReveal(expectedGen?: number): void {
  if (expectedGen != null && pending?.gen !== expectedGen) return;
  if (pending === null) return;
  pending = null;
  emit();
}

function sessionIdFromPanel(
  component: string | undefined,
  panelId: string,
): string | null {
  const prefix =
    component === "agent"
      ? "agent-"
      : component === "subagent"
        ? "subagent-"
        : null;
  if (!prefix || !panelId.startsWith(prefix)) return null;
  return panelId.slice(prefix.length) || null;
}

onPanelRemoved((event) => {
  const sid = sessionIdFromPanel(event.component, event.id);
  if (!sid) return;
  clearFoldCardOpen(sid);
  if (event.component === "agent") {
    useConnectionStore.getState().unsubscribeSession(sid);
  }
});

/** Tab/panel title for a session panel with no preview to summarize yet. A
 *  writable root reads "NEW" — a freshly created session has no message to
 *  summarize, and an id slice names nothing the user can recognize. Everything
 *  else (a subagent child, an unclassified id) keeps the short id so it stays
 *  traceable. */
export function fallbackSessionTitle(
  sessionId: string,
  isRoot: boolean,
): string {
  return isRoot ? "NEW" : sessionId.slice(0, 8);
}

function subscribe(sessionId: string): void {
  void useConnectionStore
    .getState()
    .ensureSubscribe(sessionId)
    .catch(() => {});
}

/** Open or focus the writable agent panel for `sessionId`. Optional seq is revealed after load.
 *
 * This is the TRUSTED writable/root-only entry: callers are SessionList (already
 * filtered to roots), `newSession`, and `openKnownSessionPanel` after it has
 * CONFIRMED the id is a root. A newly created session is not in `session/list`
 * yet, so the panel params carry an explicit `sessionKind: "root"` provenance —
 * `AgentPanel` uses it to render the writable shell immediately instead of
 * fail-closing to a blank read-only transcript. Provenance is only attached to a
 * NEWLY added panel; an already-open panel is activated without upgrading it. */
export function openSessionPanel(sessionId: string, revealSeq?: number): void {
  if (revealSeq != null) requestSeqReveal(sessionId, revealSeq);
  openPanel({
    id: `agent-${sessionId}`,
    component: "agent",
    title: fallbackSessionTitle(sessionId, true),
    tabComponent: "agent",
    params: { sessionId, sessionKind: "root" },
  });
  subscribe(sessionId);
}

/**
 * Open or focus the READ-ONLY subagent panel for `childId`. Optional seq is
 * revealed after load.
 *
 * `ensureSubscribe` is NOT refcounted, so a child must never be hosted by two
 * panels at once. A legacy/restored writable `agent-<id>` panel may already own
 * this child (old layouts, or a Search result opened before this phase); the
 * workbench activates that single host instead of adding a second one —
 * `AgentPanel` fail-closes a known child to the read-only transcript.
 */
export function openSubagentPanel(childId: string, revealSeq?: number): void {
  if (revealSeq != null) requestSeqReveal(childId, revealSeq);
  if (hasPanel(`agent-${childId}`)) {
    openPanel({
      id: `agent-${childId}`,
      component: "agent",
      title: fallbackSessionTitle(childId, false),
      tabComponent: "agent",
      params: { sessionId: childId },
    });
  } else {
    openPanel({
      id: `subagent-${childId}`,
      component: "subagent",
      title: fallbackSessionTitle(childId, false),
      tabComponent: "agent",
      params: { sessionId: childId },
    });
  }
  subscribe(childId);
}

/** Minimal session metadata the classifier needs. */
export interface SessionMeta {
  id: string;
  parent_session_id?: string | null;
}

export type SessionKind = "root" | "child" | "unknown";

/**
 * Classify a session id against the known session list. FAIL-CLOSED: a session
 * that is not (yet) in the list — including while the list is still loading —
 * reads as "unknown", which routes to the read-only panel.
 */
export function classifySession(
  sessions: readonly SessionMeta[],
  sessionId: string,
): SessionKind {
  const session = sessions.find((s) => s.id === sessionId);
  if (!session) return "unknown";
  return session.parent_session_id ? "child" : "root";
}

/**
 * Safe navigation for a session id of unknown provenance (Search title/hit):
 * a confirmed root opens the writable `agent-*` panel; a known child or an
 * unclassified id opens the read-only `subagent-*` panel. The caller supplies
 * the session-list snapshot so this module stays free of the store (which
 * imports this module).
 */
export function openKnownSessionPanel(
  sessionId: string,
  sessions: readonly SessionMeta[],
  revealSeq?: number,
): void {
  if (classifySession(sessions, sessionId) === "root") {
    openSessionPanel(sessionId, revealSeq);
  } else {
    openSubagentPanel(sessionId, revealSeq);
  }
}

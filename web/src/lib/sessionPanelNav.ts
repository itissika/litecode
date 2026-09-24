import { getDockviewApi, useConnectionStore } from "../stores/connectionStore";

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

type DockviewApi = NonNullable<ReturnType<typeof getDockviewApi>>;

/** Where a newly-added session panel should land: the first grid group, or a
 *  fresh group when the grid is empty. Shared by both panel flavours so the
 *  positioning logic is not copy-pasted. */
function gridPosition(api: DockviewApi): { referenceGroup: string } {
  const gridGroups = api.groups.filter((g) => g.api.location.type === "grid");
  if (gridGroups.length === 0) {
    const group = api.addGroup();
    return { referenceGroup: group.id };
  }
  return { referenceGroup: gridGroups[0]!.api.id };
}

/** Focus an already-open panel and (re)arm its subscription. Returns whether a
 *  panel was found. */
function activateExisting(
  api: DockviewApi,
  panelId: string,
  sessionId: string,
): boolean {
  const existing = api.getPanel(panelId);
  if (!existing) return false;
  existing.api.setActive();
  void useConnectionStore
    .getState()
    .ensureSubscribe(sessionId)
    .catch(() => {});
  return true;
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
  const api = getDockviewApi();
  if (!api) return;
  if (activateExisting(api, `agent-${sessionId}`, sessionId)) return;
  api.addPanel({
    id: `agent-${sessionId}`,
    component: "agent",
    title: sessionId.slice(0, 8),
    params: { sessionId, sessionKind: "root" },
    tabComponent: "agent",
    position: gridPosition(api),
  });
}

/**
 * Open or focus the READ-ONLY subagent panel for `childId`. Optional seq is
 * revealed after load.
 *
 * `ensureSubscribe` is NOT refcounted, so a child must never be hosted by two
 * panels at once. A legacy/restored writable `agent-<id>` panel may already own
 * this child (old layouts, or a Search result opened before this phase); we
 * activate that single host instead of adding a second one — `AgentPanel`
 * fail-closes a known child to the read-only transcript.
 */
export function openSubagentPanel(childId: string, revealSeq?: number): void {
  if (revealSeq != null) requestSeqReveal(childId, revealSeq);
  const api = getDockviewApi();
  if (!api) return;
  if (activateExisting(api, `subagent-${childId}`, childId)) return;
  if (activateExisting(api, `agent-${childId}`, childId)) return;
  api.addPanel({
    id: `subagent-${childId}`,
    component: "subagent",
    title: childId.slice(0, 8),
    params: { sessionId: childId },
    tabComponent: "agent",
    position: gridPosition(api),
  });
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

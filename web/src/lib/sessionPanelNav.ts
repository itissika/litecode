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

export function requestSeqReveal(sessionId: string, seq: number): PendingSeqReveal {
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

/** Open or focus the agent panel for `sessionId`. Optional seq is revealed after load. */
export function openSessionPanel(sessionId: string, revealSeq?: number): void {
  if (revealSeq != null) requestSeqReveal(sessionId, revealSeq);
  const api = getDockviewApi();
  if (!api) return;
  const panelId = `agent-${sessionId}`;
  const existing = api.getPanel(panelId);
  if (existing) {
    existing.api.setActive();
    void useConnectionStore.getState().ensureSubscribe(sessionId).catch(() => {});
    return;
  }
  const gridGroups = api.groups.filter((g) => g.api.location.type === "grid");
  let position: { referenceGroup: string } | undefined;
  if (gridGroups.length === 0) {
    const group = api.addGroup();
    position = { referenceGroup: group.id };
  } else {
    position = { referenceGroup: gridGroups[0]!.api.id };
  }
  api.addPanel({
    id: panelId,
    component: "agent",
    title: sessionId.slice(0, 8),
    params: { sessionId },
    tabComponent: "agent",
    position,
  });
}

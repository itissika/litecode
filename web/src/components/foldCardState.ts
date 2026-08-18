/**
 * Ephemeral open/collapsed state for FoldCards, persisted across virtual-list
 * unmounts.
 *
 * FoldCards live inside a virtualized message list (tanstack `useVirtualizer`,
 * `overscan: 5`). Only items in the viewport are mounted, so every card is
 * mounted → unmounted as the user scrolls. FoldCard's open state used to be
 * plain `useState`, which reset to collapsed on every remount. A card the user
 * had expanded would collapse on scroll-away, shrinking its bubble's height and
 * forcing the virtualizer to re-measure + reposition the whole list — the
 * "list jitters / won't scroll up" bug in long conversations.
 *
 * This module keeps the open state keyed by a *stable* FoldCard id (see
 * `MessageList.tsx`, derived from the bubble's projection key + the card's
 * slot, namespaced by session id) so an expanded card stays expanded after it
 * scrolls out of view and back, keeping its measured height stable.
 *
 * Invariant: a remounted card renders the exact state it had when it unmounted
 * (FoldCard persists on *every* change, including while streaming). This keeps
 * the remounted height equal to the height the virtualizer measured last time,
 * which is what prevents list jumps. Consequence: a card whose turn ended while
 * it was scrolled out of view stays open on remount — the auto-collapse effect
 * never ran for it, and its cached (open) measurement must stay valid.
 *
 * State is dropped per session when its panel closes (`clearFoldCardOpen`).
 */
const openState = new Map<string, boolean>();

/** Stable id → persisted open state, or `undefined` if never touched. */
export function getFoldCardOpen(id: string): boolean | undefined {
  return openState.get(id);
}

export function setFoldCardOpen(id: string, open: boolean): void {
  openState.set(id, open);
}

const openRequests = new Set<(id: string) => void>();

/** Open a mounted FoldCard (persist + notify). Used to reveal a bash view. */
export function requestFoldCardOpen(id: string): void {
  setFoldCardOpen(id, true);
  for (const notify of openRequests) notify(id);
}

export function subscribeFoldCardOpenRequest(notify: (id: string) => void): () => void {
  openRequests.add(notify);
  return () => {
    openRequests.delete(notify);
  };
}

/** Drop all state for a session (prefix is `sessionId:`). */
export function clearFoldCardOpen(sessionId: string): void {
  const prefix = `${sessionId}:`;
  for (const key of openState.keys()) {
    if (key.startsWith(prefix)) openState.delete(key);
  }
}

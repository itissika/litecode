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
 * Only explicit user intent is persisted. A card with no user choice is owned
 * by its caller's current system state, including after a virtual-list remount.
 * This prevents an old live measurement from overriding a completed group.
 *
 * State is dropped per session when its panel closes (`clearFoldCardOpen`).
 */
export type FoldCardOpenIntent = "none" | "keepopen" | "keepclosed";

/** Explicit user choice only. `none` is deliberately absent so the system owns the state. */
const openIntent = new Map<string, Exclude<FoldCardOpenIntent, "none">>();

/** Stable id → explicit user intent, or `none` when the system controls the card. */
export function getFoldCardOpenIntent(id: string): FoldCardOpenIntent {
  return openIntent.get(id) ?? "none";
}

export function setFoldCardOpenIntent(id: string, intent: FoldCardOpenIntent): void {
  if (intent === "none") openIntent.delete(id);
  else openIntent.set(id, intent);
}

const openRequests = new Set<(id: string) => void>();

/** Explicitly open a mounted FoldCard (persist + notify). Used to reveal a bash view. */
export function requestFoldCardOpen(id: string): void {
  setFoldCardOpenIntent(id, "keepopen");
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
  for (const key of openIntent.keys()) {
    if (key.startsWith(prefix)) openIntent.delete(key);
  }
}

/** Pure order edits shared by drag, keyboard, and menu actions. */

export type FolderDropPosition = "before" | "after";

/**
 * Move `sourceId` next to `targetId`. Returns null when either id is absent or
 * the order would not change, so callers can skip the RPC.
 */
export function reorderFolderIds(
  ids: readonly string[],
  sourceId: string,
  targetId: string,
  position: FolderDropPosition,
): string[] | null {
  const from = ids.indexOf(sourceId);
  if (from < 0) return null;
  const without = ids.filter((id) => id !== sourceId);
  const target = without.indexOf(targetId);
  if (target < 0) return null;
  without.splice(position === "before" ? target : target + 1, 0, sourceId);
  return without.every((id, index) => id === ids[index]) ? null : without;
}

/** Move one folder by one slot. Returns null at the edges or when absent. */
export function moveFolderId(
  ids: readonly string[],
  id: string,
  delta: -1 | 1,
): string[] | null {
  const index = ids.indexOf(id);
  const next = index + delta;
  if (index < 0 || next < 0 || next >= ids.length) return null;
  const ordered = [...ids];
  [ordered[index], ordered[next]] = [ordered[next], ordered[index]];
  return ordered;
}

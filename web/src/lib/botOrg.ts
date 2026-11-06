import type { BotInfo } from "../api/types";
import { botFolderId, botParentId } from "./botDirectory";

/**
 * Direct children, in directory order. Placement is one edge per bot, so a
 * child never appears twice.
 */
export function botChildren(bots: readonly BotInfo[]): Map<string, BotInfo[]> {
  const byId = new Set(bots.map((bot) => bot.id));
  const children = new Map<string, BotInfo[]>();
  for (const bot of bots) {
    const parent = botParentId(bot);
    if (!parent || parent === bot.id || !byId.has(parent)) continue;
    const bucket = children.get(parent) ?? [];
    bucket.push(bot);
    children.set(parent, bucket);
  }
  return children;
}

/**
 * Rows that start a tree. A folder bucket contains its folder roots; null is
 * the ungrouped bucket. A dangling parent is shown at the ungrouped root so a
 * stale record never disappears.
 */
export function botRoots(
  bots: readonly BotInfo[],
  folderId: string | null,
): BotInfo[] {
  const byId = new Set(bots.map((bot) => bot.id));
  return bots.filter((bot) => {
    const parent = botParentId(bot);
    if (parent && parent !== bot.id && byId.has(parent)) return false;
    const folder = botFolderId(bot);
    return folderId === null ? folder === null : folder === folderId;
  });
}

/** Depth-first walk with a guard for a hand-edited/cyclic registry. */
export function botSubtree(
  bot: BotInfo,
  children: Map<string, BotInfo[]>,
  seen: Set<string> = new Set(),
): BotInfo[] {
  if (seen.has(bot.id)) return [];
  seen.add(bot.id);
  const out: BotInfo[] = [bot];
  for (const child of children.get(bot.id) ?? []) {
    out.push(...botSubtree(child, children, seen));
  }
  return out;
}

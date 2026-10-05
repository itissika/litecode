import type { HumanRow } from "../../api/types";

/** `py-4` + dot `mt-[7px]` + half of `h-1.5`, matching ItemBubble. */
export const USER_DOT_CENTER_Y = 16 + 7 + 3;

/**
 * Index radius of the rail window. ±this many user messages fill the band,
 * and the anchor request asks for the same radius on each side of center.
 */
export const USER_RAIL_RADIUS = 2;

/** Pixel height of the centered tick band. It does not grow with the panel. */
export const USER_RAIL_BAND = 72;

/**
 * Left content inset = the rail strip: a 12px outer padding, the elastic tick
 * area, and an 8px padding that doubles as the gap to the messages. The strip
 * is content-box, so its outer box is `PAD_LEFT + width + PAD_RIGHT`:
 *
 *   min  12 + 4 + 8 = 24px
 *   max  12 + 12 + 8 = 32px → row budget reserved by AgentPanel
 */
export const USER_RAIL_PAD_LEFT = 12;
export const USER_RAIL_PAD_RIGHT = 8;

/**
 * Decorative 1px hairline above and below the tick band: a short segment, set
 * off from the ticks by USER_RAIL_SPINE_GAP.
 */
export const USER_RAIL_SPINE_LENGTH = 12;
export const USER_RAIL_SPINE_GAP = 8;
export const USER_RAIL_WIDTH_MIN = 4;
export const USER_RAIL_WIDTH_MAX = 12;

export interface UserRailLayoutMark {
  seq: number;
  /** Dot center in the transcript scroll content. */
  contentY: number;
}

const TICK_WIDTH_MAX = 16;
const TICK_WIDTH_MIN = 6;
const TICK_OPACITY_MAX = 1;
const TICK_OPACITY_MIN = 0.18;

export interface TickAppearance {
  width: number;
  opacity: number;
}

/**
 * Width and opacity from distance to the midline. `t` is 0 at the center
 * and 1 at the band edge. Emphasis is closeness squared, so a tick one
 * step out (`t` = 0.5) keeps a quarter of the span instead of half.
 */
export function tickAppearance(
  distancePx: number,
  halfViewport: number,
): TickAppearance {
  const span = Math.max(halfViewport, 1);
  const t = Math.min(1, Math.abs(distancePx) / span);
  const emphasis = (1 - t) * (1 - t);
  return {
    width: TICK_WIDTH_MIN + emphasis * (TICK_WIDTH_MAX - TICK_WIDTH_MIN),
    opacity: TICK_OPACITY_MIN + emphasis * (TICK_OPACITY_MAX - TICK_OPACITY_MIN),
  };
}

export interface RailTick {
  seq: number;
  /** Y in the tick band, 0 at the top. */
  railY: number;
}

/**
 * Even index spacing. ±RADIUS fills `bandHeight`, and the viewport focus
 * stays at half that height.
 */
export function placeUserRailTicks(
  marks: readonly UserRailLayoutMark[],
  focusContentY: number,
  bandHeight: number,
): RailTick[] {
  if (bandHeight <= 0 || marks.length === 0) return [];
  const sorted = [...marks].sort(
    (a, b) => a.contentY - b.contentY || a.seq - b.seq,
  );
  const centerIndex = fractionalIndex(sorted, focusContentY);
  const mid = bandHeight / 2;
  const pitch = bandHeight / (2 * USER_RAIL_RADIUS);
  return sorted.map((mark, index) => ({
    seq: mark.seq,
    railY: mid + (index - centerIndex) * pitch,
  }));
}

/**
 * Index of `focus`, linear inside each content gap.
 * Past either end the index stays on the first or last user message.
 */
function fractionalIndex(
  sorted: readonly UserRailLayoutMark[],
  focus: number,
): number {
  const count = sorted.length;
  if (count <= 1) return 0;
  if (focus <= sorted[0]!.contentY) return 0;
  const last = count - 1;
  if (focus >= sorted[last]!.contentY) return last;
  let index = 0;
  while (index + 1 < count && sorted[index + 1]!.contentY <= focus) index += 1;
  const gap = sorted[index + 1]!.contentY - sorted[index]!.contentY;
  return gap > 0 ? index + (focus - sorted[index]!.contentY) / gap : index;
}

/** User message whose dot is closest to the viewport midline. */
export function nearestUserMark(
  marks: readonly UserRailLayoutMark[],
  focusContentY: number,
): UserRailLayoutMark | null {
  let best: UserRailLayoutMark | null = null;
  let bestDist = Infinity;
  for (const mark of marks) {
    const dist = Math.abs(mark.contentY - focusContentY);
    if (dist < bestDist) {
      best = mark;
      bestDist = dist;
    }
  }
  return best;
}

/** Highest sealed `item/user` seq. */
export function newestUserSeq(rows: readonly HumanRow[]): number | null {
  let newest: number | null = null;
  for (const row of rows) {
    if (row.kind !== "item/user" || row.seq < 0) continue;
    if (newest == null || row.seq > newest) newest = row.seq;
  }
  return newest;
}

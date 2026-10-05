/** Tick-click reveal. Search hits keep the instant jump. */
export type RevealSeq = (
  seq: number,
  options?: { glide?: boolean },
) => void;

const GLIDE_MIN_MS = 340;
const GLIDE_MAX_MS = 720;

/**
 * Ease-in-out cubic. The start and end are slow; the middle is fastest.
 * `t` is clamped to 0..1.
 */
export function easeInOutCubic(t: number): number {
  const p = Math.min(1, Math.max(0, t));
  return p < 0.5 ? 4 * p * p * p : 1 - (-2 * p + 2) ** 3 / 2;
}

/** Scroll offset at `elapsed` along an ease-in-out from `from` to `to`. */
export function glideScrollTop(input: {
  from: number;
  to: number;
  elapsed: number;
  duration: number;
}): number {
  const p =
    input.duration <= 0 ? 1 : Math.min(1, Math.max(0, input.elapsed / input.duration));
  return input.from + (input.to - input.from) * easeInOutCubic(p);
}

/**
 * How long a glide takes. Farther targets get more time, with a floor so a
 * short move is still readable and a cap so a long one does not drag.
 */
export function glideDuration(distancePx: number): number {
  const distance = Math.abs(distancePx);
  return Math.min(GLIDE_MAX_MS, Math.max(GLIDE_MIN_MS, 260 + distance * 0.18));
}

/**
 * New `from` so `from + (nextTo - from) * eased` stays at `current`.
 * Used when the virtualizer's target offset moves mid-glide (a measurement)
 * without teleporting the viewport.
 */
export function rebaseGlideFrom(
  current: number,
  nextTo: number,
  eased: number,
): number {
  if (eased >= 0.999) return current;
  return (current - nextTo * eased) / (1 - eased);
}

/**
 * Pixels to add to one scroller so the element sits on that scroller's midline.
 * Positive moves the content up. This replaces `scrollIntoView({ block: "center" })`,
 * which also scrolls every ancestor — including `overflow: hidden` shells such as
 * dockview group views.
 */
export function centerDelta(input: {
  scrollerTop: number;
  scrollerHeight: number;
  elementTop: number;
  elementHeight: number;
}): number {
  const viewMid = input.scrollerTop + input.scrollerHeight / 2;
  const elementMid = input.elementTop + input.elementHeight / 2;
  return elementMid - viewMid;
}

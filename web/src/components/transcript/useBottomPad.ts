import { useEffect, useLayoutEffect, useRef, useState, type RefObject } from "react";

import { hostResizeObserver, viewOf } from "../../lib/domView";

/**
 * Trailing clearance, as a fraction of the scroll viewport, that keeps the tail
 * of the transcript readable: half a viewport while the composer floats over it,
 * and a fifth once the composer is collapsed (enough to keep the last line off
 * the very bottom edge, clear of the dock's collapsed toggle).
 */
const PAD_EXPANDED = 0.5;
const PAD_COLLAPSED = 0.2;

/** Native glide for the scroll moves the list makes on its own (pad changes). */
export function scrollGlide(view: Window = window): ScrollBehavior {
  return view.matchMedia("(prefers-reduced-motion: reduce)").matches
    ? "auto"
    : "smooth";
}

/**
 * Composer clearance under the transcript.
 *
 * Call `useBottomPad` first (it owns the applied pad), build the virtualizer
 * with `bottomPad`, then call `useBottomPadMotion` so the glide can move that
 * virtualizer. Dropping the pad shrinks the scroll range; the motion hook
 * glides to the new end before the pad is removed, so the browser never clamps
 * `scrollTop` by the difference.
 */
export function useBottomPad({
  scrollRef,
  composerCollapsed,
}: {
  scrollRef: RefObject<HTMLDivElement | null>;
  composerCollapsed: boolean;
}) {
  const [viewportH, setViewportH] = useState(0);
  // Pad currently *applied*. Collapsing lowers the scrollable range, so this
  // trails `composerCollapsed` by one animation.
  const [padCollapsed, setPadCollapsed] = useState(composerCollapsed);
  // Set when the pad grows (composer expanding) so the pin glides instead of
  // snapping the tail up by the extra clearance in a single frame.
  const padGrewRef = useRef(false);

  useEffect(() => {
    const el = scrollRef.current;
    const view = viewOf(el);
    const measure = () => {
      const node = scrollRef.current;
      if (!node) return;
      const h = node.getBoundingClientRect().height;
      setViewportH(h > 0 ? h : 0);
    };
    measure();
    const raf = view.requestAnimationFrame(measure);
    const Observer = hostResizeObserver(el);
    const ro = Observer ? new Observer(measure) : null;
    if (el && ro) ro.observe(el);
    return () => {
      view.cancelAnimationFrame(raf);
      ro?.disconnect();
    };
  }, [scrollRef]);

  const bottomPad = viewportH * (padCollapsed ? PAD_COLLAPSED : PAD_EXPANDED);
  const padGone = (PAD_EXPANDED - PAD_COLLAPSED) * viewportH;

  return { bottomPad, padCollapsed, setPadCollapsed, padGrewRef, padGone };
}

export function useBottomPadMotion({
  scrollRef,
  composerCollapsed,
  padCollapsed,
  setPadCollapsed,
  padGrewRef,
  padGone,
  virtualizer,
  setStick,
}: {
  scrollRef: RefObject<HTMLDivElement | null>;
  composerCollapsed: boolean;
  padCollapsed: boolean;
  setPadCollapsed: (next: boolean) => void;
  padGrewRef: { current: boolean };
  padGone: number;
  virtualizer: {
    isAtEnd: () => boolean;
    scrollToOffset: (
      offset: number,
      opts?: { behavior?: ScrollBehavior },
    ) => void;
  };
  setStick: (next: boolean) => void;
}) {
  // Arriving at the smaller pad's end means the reader is now at the end, and
  // nobody scrolled there: adopt the end as the stick intent, or the Latest
  // chip would show a lie and the next append would refuse to follow.
  const padRef = useRef(padCollapsed);
  useLayoutEffect(() => {
    if (padRef.current === padCollapsed) return;
    padRef.current = padCollapsed;
    if (virtualizer.isAtEnd()) setStick(true);
  }, [padCollapsed, setStick, virtualizer]);

  useEffect(() => {
    if (padCollapsed === composerCollapsed) return;
    const el = scrollRef.current;
    // Where the reader will end up once the smaller pad is in place.
    const nextMax = el
      ? Math.max(el.scrollHeight - el.clientHeight - padGone, 0)
      : 0;
    if (!composerCollapsed || !el || el.scrollTop <= nextMax) {
      // Expanding only ever grows the range, so it applies at once; the pin then
      // glides to the end the taller pad asks for.
      padGrewRef.current = !composerCollapsed;
      setPadCollapsed(composerCollapsed);
      return;
    }
    const view = viewOf(el);
    let settled = false;
    const finish = () => {
      if (settled) return;
      settled = true;
      el.removeEventListener("scrollend", finish);
      view.clearTimeout(fallback);
      setPadCollapsed(true);
    };
    // `scrollend` is the exact signal, the timer the safety net for engines that
    // never send it (a late drop only clamps by the few px still in flight).
    const fallback = view.setTimeout(finish, 700);
    el.addEventListener("scrollend", finish);
    virtualizer.scrollToOffset(nextMax, { behavior: scrollGlide(view) });
    return () => {
      settled = true;
      el.removeEventListener("scrollend", finish);
      view.clearTimeout(fallback);
    };
  }, [
    composerCollapsed,
    padCollapsed,
    padGone,
    padGrewRef,
    scrollRef,
    setPadCollapsed,
    virtualizer,
  ]);
}

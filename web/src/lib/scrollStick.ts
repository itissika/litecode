import {
  useCallback,
  useEffect,
  useRef,
  type MutableRefObject,
  type RefObject,
} from "react";

/** Keys that scroll a container; used to detect keyboard scroll intent. */
export const SCROLL_INTENT_KEYS = new Set([
  "ArrowUp",
  "ArrowDown",
  "PageUp",
  "PageDown",
  "Home",
  "End",
  " ",
]);

/**
 * True when a wheel/touch event originated inside a nested scroll container
 * (other than `root`). The message list and FoldCards use this so scrolling an
 * inner scroller (a FoldCard, a bash output pre) never unpins the outer one.
 */
export function wheelTargetIsNestedScroller(
  target: EventTarget | null,
  root: HTMLElement,
): boolean {
  let node = target instanceof HTMLElement ? target : null;
  while (node && node !== root) {
    const overflowY = getComputedStyle(node).overflowY;
    if (
      (overflowY === "auto" || overflowY === "scroll") &&
      node.scrollHeight > node.clientHeight + 1
    ) {
      return true;
    }
    node = node.parentElement;
  }
  return false;
}

/** Distance from the scroll end at which we consider the user "at the bottom"
 *  for RE-pinning. Unpinning is threshold-free (any scroll-up gesture unpins),
 *  so a generous re-pin band is safe — it can never re-introduce the snap-back
 *  race. 32px matches the original FoldCard stick leniency; tighter values
 *  (e.g. 8px) made wheel re-pin miss on trackpads that settle short. */
const REPIN_THRESHOLD = 32;

/** True when `el` is within `threshold` px of its scroll end. */
export function isNearScrollEnd(
  el: HTMLElement,
  threshold = REPIN_THRESHOLD,
): boolean {
  return el.scrollHeight - el.scrollTop - el.clientHeight <= threshold;
}

/**
 * Bottom-stick state + gesture listeners for a scroll container.
 *
 * This is the message list's stick model, extracted so FoldCards and live
 * output pre's can reuse it:
 *
 *  - The stick flag is an authoritative ref, NOT recomputed from the scroll
 *    position on every scroll event.
 *  - Unpin happens *synchronously* on a scroll-up gesture (wheel-up /
 *    touch-drag-down / ArrowUp/PageUp/Home), so a stream flush landing in the
 *    same frame cannot race ahead of the user's intent and yank them back to
 *    the bottom.
 *  - Re-pin is conservative: only when the user scrolls back to the end
 *    (checked in a rAF after a scroll-down gesture, or on a scrollbar drag).
 *    Streaming flips never re-pin an unpinned container.
 *
 * The returned `stickRef` is stable; the caller's own effects (e.g. a layout
 * effect that pins `scrollTop` when the ref is true) may read it.
 */
export function useStickToBottom({
  ref,
  active,
  initialStick = false,
  isAtEnd,
  onStickChange,
  onScroll,
}: {
  ref: RefObject<HTMLElement | null>;
  /** Bind listeners only while true (e.g. a FoldCard's body is mounted). */
  active: boolean;
  initialStick?: boolean;
  /** Recompute stick from the current scroll position (scroll-down / scrollbar). */
  isAtEnd: () => boolean;
  /** Called whenever the stick flag changes (for state sync / parent notify). */
  onStickChange?: (stick: boolean) => void;
  /** Extra work on each scroll (e.g. recompute edge blur). */
  onScroll?: () => void;
}): { stickRef: MutableRefObject<boolean>; setStick: (next: boolean) => void } {
  const stick = useRef(initialStick);
  const isAtEndRef = useRef(isAtEnd);
  isAtEndRef.current = isAtEnd;
  const onStickChangeRef = useRef(onStickChange);
  onStickChangeRef.current = onStickChange;
  const onScrollRef = useRef(onScroll);
  onScrollRef.current = onScroll;

  const setStick = useCallback((next: boolean) => {
    if (stick.current === next) return;
    stick.current = next;
    onStickChangeRef.current?.(next);
  }, []);

  useEffect(() => {
    const el = ref.current;
    if (!el || !active) return;

    let stickRaf = 0;
    let scrollRaf = 0;

    // Re-check after a scroll-down gesture / scrollbar release. Deferred to a
    // rAF so trackpad momentum that settles past the end does not flip-flop.
    const afterHumanScroll = () => {
      if (stickRaf) return;
      stickRaf = requestAnimationFrame(() => {
        stickRaf = 0;
        setStick(isAtEndRef.current());
      });
    };

    const onWheel = (event: WheelEvent) => {
      if (wheelTargetIsNestedScroller(event.target, el)) return;
      if (event.deltaY < 0) {
        setStick(false);
        return;
      }
      afterHumanScroll();
    };

    let touchY = 0;
    const onTouchStart = (event: TouchEvent) => {
      touchY = event.touches[0]?.clientY ?? 0;
    };
    const onTouchMove = (event: TouchEvent) => {
      if (wheelTargetIsNestedScroller(event.target, el)) return;
      const y = event.touches[0]?.clientY ?? touchY;
      const dy = y - touchY;
      touchY = y;
      if (dy > 2) {
        setStick(false);
        return;
      }
      if (dy < -2) afterHumanScroll();
    };

    const onKeyDown = (event: KeyboardEvent) => {
      if (!SCROLL_INTENT_KEYS.has(event.key)) return;
      if (event.key === "ArrowUp" || event.key === "PageUp" || event.key === "Home") {
        setStick(false);
        return;
      }
      afterHumanScroll();
    };

    let fromScrollbar = false;
    const onPointerDown = (event: PointerEvent) => {
      fromScrollbar = event.target === el;
    };
    const onPointerUp = () => {
      if (!fromScrollbar) return;
      fromScrollbar = false;
      afterHumanScroll();
    };
    const onScroll = () => {
      if (fromScrollbar) {
        setStick(isAtEndRef.current());
      }
      if (onScrollRef.current && !scrollRaf) {
        scrollRaf = requestAnimationFrame(() => {
          scrollRaf = 0;
          onScrollRef.current?.();
        });
      }
    };

    el.addEventListener("wheel", onWheel, { passive: true });
    el.addEventListener("touchstart", onTouchStart, { passive: true });
    el.addEventListener("touchmove", onTouchMove, { passive: true });
    el.addEventListener("keydown", onKeyDown);
    el.addEventListener("pointerdown", onPointerDown);
    el.addEventListener("pointerup", onPointerUp);
    el.addEventListener("scroll", onScroll, { passive: true });
    return () => {
      if (stickRaf) cancelAnimationFrame(stickRaf);
      if (scrollRaf) cancelAnimationFrame(scrollRaf);
      el.removeEventListener("wheel", onWheel);
      el.removeEventListener("touchstart", onTouchStart);
      el.removeEventListener("touchmove", onTouchMove);
      el.removeEventListener("keydown", onKeyDown);
      el.removeEventListener("pointerdown", onPointerDown);
      el.removeEventListener("pointerup", onPointerUp);
      el.removeEventListener("scroll", onScroll);
    };
  }, [ref, active, setStick]);

  return { stickRef: stick, setStick };
}

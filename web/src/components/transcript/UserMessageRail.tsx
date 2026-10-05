import { useEffect, useLayoutEffect, useRef, type RefObject } from "react";

import {
  nearestUserMark,
  placeUserRailTicks,
  tickAppearance,
  USER_RAIL_BAND,
  USER_RAIL_PAD_LEFT,
  USER_RAIL_PAD_RIGHT,
  USER_RAIL_SPINE_GAP,
  USER_RAIL_SPINE_LENGTH,
  USER_RAIL_WIDTH_MAX,
  USER_RAIL_WIDTH_MIN,
  type UserRailLayoutMark,
} from "./transcriptUserRailMarks";

export interface UserMessageRailProps {
  sessionId: string;
  scrollRef: RefObject<HTMLDivElement | null>;
  /** Dot centers written by the transcript. The rail does not subscribe to rows. */
  layoutRef: RefObject<UserRailLayoutMark[]>;
  /** Fired after the transcript publishes a new layout. */
  notifyRef: RefObject<(() => void) | null>;
  /** History still exists above the loaded window. */
  canLoadMore: boolean;
  onCenterSeq: (seq: number) => void;
  onJump: (seq: number) => void;
}

/** Hit box stays usable when the visual tick and the strip are only a few pixels. */
const TICK_HIT_MIN = 12;

/**
 * Tick look. Re-applied on every paint: the buttons are created imperatively
 * and outlive renders, so a class edit would otherwise only reach ticks
 * created after a reload.
 */
const TICK_CLASS =
  "pointer-events-auto absolute right-0 box-content h-0.5 origin-right -translate-y-1/2 rounded-full bg-(--_dk-text-muted) py-[5px] transition-transform duration-100 [background-clip:content-box] hover:scale-110 hover:bg-(--_dk-text-secondary) active:scale-90 active:bg-(--_dk-text-disabled)";

/**
 * User-tick strip beside the message list. Its tick area is elastic
 * (`clamp(4px, 3cqw, 12px)` of the scroll viewport, content-box) with a 12px
 * outer padding and an 8px padding toward the messages. Short 1px hairlines
 * sit above and below the tick band in the theme's lowest line tier. The
 * band stays centered on the scroll viewport and does not clip the hit box.
 */
export function UserMessageRail({
  sessionId,
  scrollRef,
  layoutRef,
  notifyRef,
  canLoadMore,
  onCenterSeq,
  onJump,
}: UserMessageRailProps) {
  const rootRef = useRef<HTMLDivElement>(null);
  const topSpineRef = useRef<HTMLDivElement>(null);
  const bottomSpineRef = useRef<HTMLDivElement>(null);
  const onCenterRef = useRef(onCenterSeq);
  const onJumpRef = useRef(onJump);
  const lastCenter = useRef<number | null>(null);
  const canLoadMoreRef = useRef(canLoadMore);
  onCenterRef.current = onCenterSeq;
  onJumpRef.current = onJump;
  canLoadMoreRef.current = canLoadMore;

  // MessageList publishes marks from a layout effect. This rail is the
  // earlier sibling, so the reset lands before that publish and the frame
  // that would otherwise reuse the previous session's center seq.
  useLayoutEffect(() => {
    lastCenter.current = null;
  }, [sessionId]);

  useLayoutEffect(() => {
    notifyRef.current?.();
  }, [canLoadMore, notifyRef]);

  useEffect(() => {
    const scroller = scrollRef.current;
    const root = rootRef.current;
    if (!scroller || !root) return;

    let frame = 0;
    const paint = () => {
      frame = 0;
      const height = root.clientHeight;
      const viewHeight = scroller.clientHeight;
      if (height <= 0 || viewHeight <= 0) return;
      // A direction with nothing left to scroll to loses its decorative
      // hairline; a transcript that fits hides both.
      const maxScroll = scroller.scrollHeight - viewHeight;
      if (topSpineRef.current) {
        topSpineRef.current.style.opacity =
          canLoadMoreRef.current || scroller.scrollTop > 0 ? "1" : "0";
      }
      if (bottomSpineRef.current) {
        bottomSpineRef.current.style.opacity =
          scroller.scrollTop >= maxScroll - 1 ? "0" : "1";
      }
      const marks = layoutRef.current ?? [];
      const focus = scroller.scrollTop + viewHeight / 2;
      const nearest = nearestUserMark(marks, focus);
      if (nearest && nearest.seq !== lastCenter.current) {
        lastCenter.current = nearest.seq;
        onCenterRef.current(nearest.seq);
      }

      const ticks = placeUserRailTicks(marks, focus, height);
      const drawn = new Set<number>();
      for (const tick of ticks) {
        if (tick.railY < -24 || tick.railY > height + 24) continue;
        drawn.add(tick.seq);
        let node = root.querySelector<HTMLButtonElement>(
          `[data-user-rail-seq="${tick.seq}"]`,
        );
        if (!node) {
          node = document.createElement("button");
          node.type = "button";
          node.dataset.userRailSeq = String(tick.seq);
          node.setAttribute("aria-label", "Jump to user message");
          node.addEventListener("click", () => {
            const seq = Number(node!.dataset.userRailSeq);
            if (Number.isFinite(seq)) onJumpRef.current(seq);
          });
          root.appendChild(node);
        }
        if (node.className !== TICK_CLASS) node.className = TICK_CLASS;
        const look = tickAppearance(tick.railY - height / 2, height / 2);
        node.style.display = "";
        const hit = Math.max(look.width, TICK_HIT_MIN);
        node.style.top = `${tick.railY}px`;
        node.style.width = `${hit}px`;
        node.style.backgroundSize = `${look.width}px 100%`;
        node.style.backgroundRepeat = "no-repeat";
        node.style.backgroundPosition = "right center";
        node.style.opacity = String(look.opacity);
      }
      for (const node of root.querySelectorAll<HTMLElement>("[data-user-rail-seq]")) {
        const seq = Number(node.dataset.userRailSeq);
        if (!drawn.has(seq)) node.style.display = "none";
      }
    };

    const schedule = () => {
      if (frame !== 0) return;
      frame = requestAnimationFrame(paint);
    };

    notifyRef.current = schedule;
    scroller.addEventListener("scroll", schedule, { passive: true });
    const observer =
      typeof ResizeObserver === "undefined"
        ? null
        : new ResizeObserver(schedule);
    observer?.observe(scroller);
    schedule();

    return () => {
      if (notifyRef.current === schedule) notifyRef.current = null;
      scroller.removeEventListener("scroll", schedule);
      observer?.disconnect();
      if (frame !== 0) cancelAnimationFrame(frame);
    };
  }, [scrollRef, layoutRef, notifyRef]);

  return (
    <aside
      aria-label="User messages"
      className="box-content pointer-events-none shrink-0"
      style={{
        width: `clamp(${USER_RAIL_WIDTH_MIN}px, 3cqw, ${USER_RAIL_WIDTH_MAX}px)`,
        paddingLeft: USER_RAIL_PAD_LEFT,
        paddingRight: USER_RAIL_PAD_RIGHT,
      }}
    >
      {/* A viewport-tall sticky track that centers the band; the two short
          hairlines are set off from the ticks by the flex gap. */}
      <div
        className="pointer-events-none sticky flex w-full flex-col items-end justify-center"
        style={{ top: 0, height: "100cqh", gap: USER_RAIL_SPINE_GAP }}
      >
        <div
          ref={topSpineRef}
          className="w-px bg-(--_dk-line) transition-opacity duration-150"
          style={{ height: USER_RAIL_SPINE_LENGTH }}
        />
        <div
          ref={rootRef}
          data-testid="transcript-user-rail"
          className="relative w-full"
          style={{ height: USER_RAIL_BAND }}
        />
        <div
          ref={bottomSpineRef}
          className="w-px bg-(--_dk-line) transition-opacity duration-150"
          style={{ height: USER_RAIL_SPINE_LENGTH }}
        />
      </div>
    </aside>
  );
}

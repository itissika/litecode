import { useEffect, useRef, type RefObject } from "react";

import {
  nearestUserMark,
  placeUserRailTicks,
  tickAppearance,
  USER_RAIL_BAND,
  type UserRailLayoutMark,
} from "./transcriptUserRailMarks";

/** Clear space between a tick's right end and the user-message dot. */
const RAIL_DOT_GAP = 8;

export interface UserMessageRailProps {
  scrollRef: RefObject<HTMLDivElement | null>;
  /** Message column. Ticks sit to the left of its left edge. */
  columnRef: RefObject<HTMLElement | null>;
  /** Dot centers written by the transcript. The rail does not subscribe to rows. */
  layoutRef: RefObject<UserRailLayoutMark[]>;
  /** Fired after the transcript publishes a new layout. */
  notifyRef: RefObject<(() => void) | null>;
  onCenterSeq: (seq: number) => void;
  onJump: (seq: number) => void;
  className?: string;
}

/**
 * Fixed-pitch user ticks in a short band centered on the transcript.
 * The band sits just left of the message column, clear of the user dots,
 * and ticks grow left. The viewport midline stays at the middle of the band.
 */
export function UserMessageRail({
  scrollRef,
  columnRef,
  layoutRef,
  notifyRef,
  onCenterSeq,
  onJump,
  className = "",
}: UserMessageRailProps) {
  const hostRef = useRef<HTMLDivElement>(null);
  const rootRef = useRef<HTMLDivElement>(null);
  const onCenterRef = useRef(onCenterSeq);
  const onJumpRef = useRef(onJump);
  const lastCenter = useRef<number | null>(null);
  onCenterRef.current = onCenterSeq;
  onJumpRef.current = onJump;

  useEffect(() => {
    const scroller = scrollRef.current;
    const host = hostRef.current;
    const root = rootRef.current;
    if (!scroller || !host || !root) return;

    const placeBand = () => {
      const column = columnRef.current;
      if (!column) return;
      const hostBox = host.getBoundingClientRect();
      const columnBox = column.getBoundingClientRect();
      root.style.right = `${hostBox.right - (columnBox.left - RAIL_DOT_GAP)}px`;
    };

    let frame = 0;
    const paint = () => {
      frame = 0;
      placeBand();
      const height = root.clientHeight;
      const viewHeight = scroller.clientHeight;
      if (height <= 0 || viewHeight <= 0) return;
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
          node.className =
            "pointer-events-auto absolute right-0 box-content h-0.5 -translate-y-1/2 rounded-full bg-(--_dk-accent-hover) py-[5px] transition-none [background-clip:content-box]";
          node.style.transition = "none";
          node.addEventListener("click", () => {
            const seq = Number(node!.dataset.userRailSeq);
            if (Number.isFinite(seq)) onJumpRef.current(seq);
          });
          root.appendChild(node);
        }
        const look = tickAppearance(tick.railY - height / 2, height / 2);
        node.style.display = "";
        node.style.top = `${tick.railY}px`;
        node.style.width = `${look.width}px`;
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
    const column = columnRef.current;
    if (column) observer?.observe(column);
    schedule();

    return () => {
      if (notifyRef.current === schedule) notifyRef.current = null;
      scroller.removeEventListener("scroll", schedule);
      observer?.disconnect();
      if (frame !== 0) cancelAnimationFrame(frame);
    };
  }, [scrollRef, columnRef, layoutRef, notifyRef]);

  return (
    <div ref={hostRef} className={`pointer-events-none ${className}`}>
      <div
        ref={rootRef}
        data-testid="transcript-user-rail"
        className="pointer-events-none absolute top-1/2 w-4 -translate-y-1/2 overflow-hidden"
        style={{ height: USER_RAIL_BAND }}
      />
    </div>
  );
}

import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type RefObject,
} from "react";
import { useVirtualizer } from "@tanstack/react-virtual";

import {
  bubbleImageCount,
  bubblePlainText,
  estimateAssistantBubbleHeight,
  estimateUserBubbleHeight,
  locateBashTool,
  locateSeq,
  type Bubble,
} from "../../lib/transcriptProjection";
import { SCROLL_INTENT_KEYS, useStickToBottom } from "../../lib/scrollStick";
import { requestFoldCardOpen } from "../foldCardState";
import {
  centerDelta,
  easeInOutCubic,
  glideDuration,
  rebaseGlideFrom,
  type RevealSeq,
} from "./transcriptScrollGlide";
import { hostHtmlElement, viewOf } from "../../lib/domView";
import { scrollGlide } from "./useBottomPad";
import {
  followScrollerWindow,
  observeRowHeight,
  rebindVirtualizerWindow,
} from "./virtualizerWindow";

/** History sentinel, applied as paddingStart so it is not a virtual item. */
export const HISTORY_LOADER_HEIGHT = 40;
/** Trailing transient "compacting…" row (not a real buffer item). */
const COMPACTING_PENDING_KEY = "__compacting_pending__";
const COMPACTING_LINE_HEIGHT = 22;
/** Virtual key of the trailing queued-batch bubble. */
const QUEUE_BUBBLE_KEY = "__pending_queue__";
const MARK_ESTIMATE = 28;
const EDITING_ESTIMATE = 240;

/**
 * Whether a measured size change should be compensated by shifting scrollTop.
 *
 * The virtualizer normally compensates every first measurement, regardless of
 * where the item is. That is wrong for a newly loaded bubble below the reader:
 * its growth does not move anything in the viewport, and shifting scrollTop by
 * the full delta can pull the reader back toward the latest message. The list
 * items are absolutely positioned, so native browser anchoring cannot help.
 *
 * While unpinned, compensate only changes to items entirely above the viewport
 * (including first measurements from history paging) to preserve the visible
 * content's position. A pinned reader continues to follow all size changes.
 */
export function shouldCompensateSizeChange(input: {
  /** User is pinned to the end (bottom) of the list. */
  stickToEnd: boolean;
  /** Bottom edge of the changed item, in scroll coordinates. */
  itemEnd: number;
  /** Current scroll offset. */
  scrollOffset: number;
}): boolean {
  if (input.stickToEnd) return true;
  return input.itemEnd <= input.scrollOffset;
}

function bashCallSelector(callId: string): string {
  const escaped =
    typeof CSS !== "undefined" && typeof CSS.escape === "function"
      ? CSS.escape(callId)
      : callId.replace(/\\/g, "\\\\").replace(/"/g, '\\"');
  return `[data-bash-call-id="${escaped}"]`;
}

function seqHitSelector(seq: number): string {
  return `[data-seq-hit~="${seq}"]`;
}

/** Center `el` inside the transcript scroller only. Ancestors stay put. */
function alignInScroller(scroller: HTMLElement, el: HTMLElement): void {
  const host = scroller.getBoundingClientRect();
  const box = el.getBoundingClientRect();
  if (host.height <= 0 || box.height <= 0) return;
  const delta = centerDelta({
    scrollerTop: host.top,
    scrollerHeight: host.height,
    elementTop: box.top,
    elementHeight: box.height,
  });
  if (Math.abs(delta) < 1) return;
  scroller.scrollTop += delta;
}

export function useTranscriptViewport({
  bubbles,
  compactingRows,
  queueRows,
  queueText,
  queueImageCount = 0,
  canLoadMore,
  loadingHistory,
  onLoadMore,
  scrollRef,
  sessionId,
  paddingEnd,
  padGrewRef,
  editingBubbleKey,
  onStickChange,
  jumpToEndRef,
  revealBashRef,
  revealSeqRef,
  /** Fired after the virtualizer changes geometry, including a measurement
   *  that updates positions without a React render. */
  onGeometryRef,
}: {
  bubbles: Bubble[];
  compactingRows: number;
  queueRows: number;
  queueText: string | null;
  queueImageCount?: number;
  canLoadMore: boolean;
  loadingHistory: boolean;
  onLoadMore: () => void;
  scrollRef: RefObject<HTMLDivElement | null>;
  sessionId: string;
  paddingEnd: number;
  padGrewRef: { current: boolean };
  editingBubbleKey?: string;
  onStickChange?: (stickToEnd: boolean) => void;
  jumpToEndRef?: RefObject<(() => void) | null>;
  revealBashRef?: RefObject<((callId: string) => void) | null>;
  revealSeqRef?: RefObject<RevealSeq | null>;
  onGeometryRef?: RefObject<(() => void) | null>;
}) {
  const count = bubbles.length + compactingRows + queueRows;
  const paddingStart = canLoadMore ? HISTORY_LOADER_HEIGHT : 0;
  const bubblesRef = useRef(bubbles);
  bubblesRef.current = bubbles;
  const [stickToEnd, setStickToEnd] = useState(true);
  const onStickChangeRef = useRef(onStickChange);
  onStickChangeRef.current = onStickChange;

  const getItemKey = useCallback(
    (index: number) => {
      if (index < bubbles.length) return bubbles[index]!.key;
      return index === bubbles.length + compactingRows
        ? QUEUE_BUBBLE_KEY
        : COMPACTING_PENDING_KEY;
    },
    [bubbles, compactingRows],
  );

  const estimateSize = useCallback(
    (index: number) => {
      if (index >= bubbles.length) {
        return index === bubbles.length + compactingRows
          ? estimateUserBubbleHeight(queueText ?? "", queueImageCount)
          : COMPACTING_LINE_HEIGHT;
      }
      const bubble = bubbles[index];
      if (!bubble || bubble.markOnly || !bubble.first) return MARK_ESTIMATE;
      if (bubble.isUser) {
        return editingBubbleKey === bubble.key
          ? EDITING_ESTIMATE
          : estimateUserBubbleHeight(
              bubblePlainText(bubble),
              bubbleImageCount(bubble),
            );
      }
      return estimateAssistantBubbleHeight(bubble);
    },
    [bubbles, compactingRows, editingBubbleKey, queueImageCount, queueText],
  );

  // Human stick intent: true until the user scrolls up. The stick flag is an
  // authoritative ref driven by gestures (see useStickToBottom); React state
  // (`stickToEnd`) is synced from it for the virtualizer + Latest button.
  // Note: `isAtEnd` only closes over `virtualizer` — it is invoked from the
  // hook's gesture listeners, which run after this render, so declaring the
  // virtualizer below is safe.
  const { stickRef, setStick } = useStickToBottom({
    ref: scrollRef,
    active: true,
    initialStick: true,
    isAtEnd: () => virtualizer.isAtEnd(),
    onStickChange: useCallback((next: boolean) => {
      setStickToEnd(next);
      onStickChangeRef.current?.(next);
    }, []),
  });

  const virtualizer = useVirtualizer({
    count,
    getScrollElement: () => scrollRef.current,
    estimateSize,
    overscan: 6,
    getItemKey,
    paddingStart,
    paddingEnd,
    anchorTo: "end",
    // Measurement corrects scrollTop immediately, then tells React later.
    // Writing positions in that same callback keeps the two from landing a
    // frame apart. The container height and item transforms belong to the
    // virtualizer; MessageList must not set them.
    directDomUpdates: true,
    onChange: () => {
      onGeometryRef?.current?.();
    },
  });

  // See shouldCompensateSizeChange for the rule. While unpinned, compensate
  // only when a changed item ends above the viewport; growth below or around the
  // reader cannot move their current content anchor. `stickRef` (not the async
  // React state) is read so a stream flush in the same frame as a wheel-unpin
  // gesture still sees the unpinned intent.
  // This predicate is a public instance property, not a VirtualizerOptions field
  // in this version — assigned once per instance (idempotent on re-render, same
  // pattern as the library's own setOptions).
  virtualizer.shouldAdjustScrollPositionOnItemSizeChange = (
    item,
    _delta,
    instance,
  ) =>
    shouldCompensateSizeChange({
      stickToEnd: stickRef.current,
      itemEnd: item.end,
      scrollOffset: instance.scrollOffset ?? 0,
    });

  const virtualItems = virtualizer.getVirtualItems();
  const totalSize = virtualizer.getTotalSize();

  const pinToEnd = useCallback(() => {
    setStick(true);
    virtualizer.scrollToEnd();
  }, [setStick, virtualizer]);
  if (jumpToEndRef) jumpToEndRef.current = pinToEnd;

  const revealBash = useCallback(
    (callId: string) => {
      setStick(false);
      const view = viewOf(scrollRef.current);
      const started = view.performance.now();
      const seek = () => {
        const located = locateBashTool(bubblesRef.current, callId, sessionId);
        if (!located) {
          if (view.performance.now() - started < 800) view.requestAnimationFrame(seek);
          return;
        }
        for (const id of located.foldIds) requestFoldCardOpen(id);
        virtualizer.scrollToIndex(located.bubbleIndex, { align: "center" });
        const paintStarted = view.performance.now();
        const tick = () => {
          const scroller = scrollRef.current;
          const el = hostHtmlElement(
            (scroller ?? view.document).querySelector(bashCallSelector(callId)),
            scroller,
          );
          if (el) {
            if (scroller) alignInScroller(scroller, el);
            el.classList.remove("bash-view-reveal");
            void el.offsetWidth;
            el.classList.add("bash-view-reveal");
            return;
          }
          if (view.performance.now() - paintStarted < 800) view.requestAnimationFrame(tick);
        };
        view.requestAnimationFrame(tick);
      };
      seek();
    },
    [scrollRef, sessionId, setStick, virtualizer],
  );
  if (revealBashRef) revealBashRef.current = revealBash;

  const glideStopRef = useRef<(() => void) | null>(null);
  useEffect(() => () => glideStopRef.current?.(), []);

  const revealSeq = useCallback(
    (seq: number, options?: { glide?: boolean }) => {
      setStick(false);
      glideStopRef.current?.();
      const view = viewOf(scrollRef.current);
      const started = view.performance.now();
      const emphasize = (correct: boolean) => {
        const paintStarted = view.performance.now();
        const tick = () => {
          const scroller = scrollRef.current;
          const el = hostHtmlElement(
            (scroller ?? view.document).querySelector(seqHitSelector(seq)),
            scroller,
          );
          if (el) {
            if (correct && scroller) alignInScroller(scroller, el);
            el.classList.remove("session-search-reveal");
            void el.offsetWidth;
            el.classList.add("session-search-reveal");
            return;
          }
          if (view.performance.now() - paintStarted < 800) view.requestAnimationFrame(tick);
        };
        view.requestAnimationFrame(tick);
      };
      const seek = () => {
        const bubbleIndex = locateSeq(bubblesRef.current, seq);
        if (bubbleIndex == null) {
          if (view.performance.now() - started < 800) view.requestAnimationFrame(seek);
          return;
        }
        const scroller = scrollRef.current;
        const glide =
          options?.glide === true &&
          scroller != null &&
          scrollGlide(view) === "smooth";
        if (!glide || !scroller) {
          virtualizer.scrollToIndex(bubbleIndex, { align: "center" });
          emphasize(true);
          return;
        }
        const initial = virtualizer.getOffsetForIndex(bubbleIndex, "center")?.[0];
        if (initial == null || Math.abs(initial - scroller.scrollTop) < 2) {
          if (initial != null) scroller.scrollTop = initial;
          else virtualizer.scrollToIndex(bubbleIndex, { align: "center" });
          emphasize(true);
          return;
        }
        let from = scroller.scrollTop;
        let to = initial;
        const duration = glideDuration(to - from);
        const t0 = view.performance.now();
        let raf = 0;
        let stopped = false;
        const detach = () => {
          if (raf !== 0) view.cancelAnimationFrame(raf);
          raf = 0;
          scroller.removeEventListener("wheel", onAbort);
          scroller.removeEventListener("pointerdown", onAbort);
          view.removeEventListener("keydown", onKey);
          if (glideStopRef.current === stop) glideStopRef.current = null;
        };
        function stop() {
          stopped = true;
          detach();
        }
        function onAbort() {
          stop();
        }
        function onKey(event: KeyboardEvent) {
          if (SCROLL_INTENT_KEYS.has(event.key)) stop();
        }
        const step = (now: number) => {
          if (stopped) return;
          const p = Math.min(1, (now - t0) / duration);
          const eased = easeInOutCubic(p);
          const nextTo =
            virtualizer.getOffsetForIndex(bubbleIndex, "center")?.[0] ?? to;
          if (nextTo !== to) {
            from = rebaseGlideFrom(scroller.scrollTop, nextTo, eased);
            to = nextTo;
          }
          scroller.scrollTop = from + (to - from) * eased;
          if (p < 1) {
            raf = view.requestAnimationFrame(step);
            return;
          }
          const dest =
            virtualizer.getOffsetForIndex(bubbleIndex, "center")?.[0] ?? to;
          scroller.scrollTop = dest;
          detach();
          emphasize(false);
        };
        glideStopRef.current = stop;
        scroller.addEventListener("wheel", onAbort, { passive: true });
        scroller.addEventListener("pointerdown", onAbort);
        view.addEventListener("keydown", onKey);
        raf = view.requestAnimationFrame(step);
      };
      seek();
    },
    [scrollRef, setStick, virtualizer],
  );
  if (revealSeqRef) revealSeqRef.current = revealSeq;

  useLayoutEffect(() => {
    // Consumed either way: the flag only describes this commit's pad growth.
    const grew = padGrewRef.current;
    padGrewRef.current = false;
    if (!stickToEnd) return;
    virtualizer.scrollToEnd(
      grew ? { behavior: scrollGlide(viewOf(scrollRef.current)) } : undefined,
    );
  }, [stickToEnd, totalSize, count, virtualizer, padGrewRef]);

  // paddingStart is not an item, so a change that does not move the first
  // bubble's key (the last history page contained only hidden rows) is invisible
  // to the virtualizer's prepend anchor. Shift scrollTop by the same delta or
  // the reader jumps by the loader's height.
  const paddingRef = useRef(paddingStart);
  const countRef = useRef(count);
  const firstKeyRef = useRef(bubbles[0]?.key);
  useLayoutEffect(() => {
    const prevPad = paddingRef.current;
    const prevCount = countRef.current;
    const prevKey = firstKeyRef.current;
    const firstKey = bubbles[0]?.key;
    paddingRef.current = paddingStart;
    countRef.current = count;
    firstKeyRef.current = firstKey;
    if (prevPad === paddingStart || stickRef.current) return;
    if (prevCount !== count || prevKey !== firstKey) return;
    const el = scrollRef.current;
    if (!el) return;
    el.scrollTop = Math.max(0, el.scrollTop + (paddingStart - prevPad));
  }, [bubbles, count, paddingStart, scrollRef, stickRef]);

  useEffect(() => {
    if (!canLoadMore || loadingHistory) return;
    if (count === 0 || virtualItems.some((item) => item.index === 0)) {
      onLoadMore();
    }
  }, [canLoadMore, count, loadingHistory, onLoadMore, virtualItems]);

  // The scroller is the same node after a popout. Rebind the virtualizer onto
  // that window, and while a mini chat is open measure its row from there so
  // the editor is not covered by the next row.
  useEffect(() => {
    const scroller = scrollRef.current;
    if (!scroller) return;
    return followScrollerWindow(scroller, () => rebindVirtualizerWindow(virtualizer));
  }, [scrollRef, virtualizer]);

  useLayoutEffect(() => {
    if (!editingBubbleKey) return;
    const scroller = scrollRef.current;
    const input = scroller?.querySelector("[data-mini-chat-input]");
    const item = hostHtmlElement(input?.closest("[data-index]") ?? null, scroller);
    if (!item) return;
    const index = Number(item.dataset.index);
    if (!Number.isInteger(index)) return;
    return observeRowHeight(item, (height) => {
      virtualizer.resizeItem(index, height);
    });
  }, [editingBubbleKey, scrollRef, virtualizer]);

  return { virtualizer, virtualItems, totalSize, setStick };
}

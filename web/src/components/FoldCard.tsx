import { useCallback, useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";

import { ProgressiveBlur } from "./ProgressiveBlur";
import { getFoldCardOpen, requestFoldCardOpen, setFoldCardOpen, subscribeFoldCardOpenRequest } from "./foldCardState";

/** Marks header chrome that should brighten/dim with the FoldCard row. */
export const FOLDCARD_HEADER_TONE = "foldcard-header-tone";

interface FoldCardProps {
  /** Stable identity used to persist open/collapsed across virtual-list remounts.
   *  If omitted, state is purely local (resets on unmount). */
  id?: string;
  /** Controlled open state. If set, parent manages open/close. */
  open?: boolean;
  /** Default open state (uncontrolled), used only when the card has never been
   *  persisted (first mount, or no `id`). */
  defaultOpen?: boolean;
  onToggle?: (open: boolean) => void;
  icon?: ReactNode;
  label: ReactNode;
  /** Accessible name for the header toggle when the visual label is icon-only. */
  headerAriaLabel?: string;
  streaming?: boolean;
  className?: string;
  /** Extra classes merged onto the header row (e.g. bump summary text size). */
  headerClassName?: string;
  contentClassName?: string;
  children: ReactNode;
}

/** Scroll distance (px) from an edge at which the edge blur reaches full strength. */
const BLUR_SCALE = 140;
/** Maximum blur radius (px) passed to ProgressiveBlur at full strength (matches AgentPanel). */
const BLUR_MAX = 5;
/** Band height (px) for the top/bottom fade (matches AgentPanel). */
const BLUR_BAND = 32;
/** Distance from the inner scroller's end at which we still consider the user pinned. */
const STICK_THRESHOLD = 32;

function isNearScrollEnd(el: HTMLElement, threshold = STICK_THRESHOLD): boolean {
  return el.scrollHeight - el.scrollTop - el.clientHeight <= threshold;
}

function cssTimeToMs(value: string): number {
  const first = value.split(",")[0]?.trim() ?? "0s";
  if (first.endsWith("ms")) return Number.parseFloat(first) || 0;
  return (Number.parseFloat(first) || 0) * 1000;
}

export function FoldCard({
  id,
  open: controlledOpen,
  defaultOpen,
  onToggle,
  icon,
  label,
  headerAriaLabel,
  streaming,
  className = "",
  headerClassName = "",
  contentClassName = "",
  children,
}: FoldCardProps) {
  // Open/collapsed state is persisted under `id` on EVERY change (including
  // while streaming), so a remount — virtual-list scroll-out/in — restores the
  // exact state the card had when it unmounted. Keeping the remounted height
  // equal to the height the virtualizer measured last time is what prevents
  // list jumps. The persisted value is authoritative:
  //   - a card the user collapsed (even mid-stream) stays collapsed on remount;
  //   - a card whose turn ended while scrolled out of view stays open — the
  //     auto-collapse effect never ran for it, and reopening keeps its measured
  //     height stable (mounting it collapsed instead would re-measure short and
  //     jitter the list).
  // `defaultOpen` / `streaming` only supply a fallback for cards that have
  // never been persisted (first mount).
  const persisted = id !== undefined ? getFoldCardOpen(id) : undefined;
  const initialOpen =
    controlledOpen !== undefined
      ? controlledOpen
      : persisted !== undefined
        ? persisted
        : defaultOpen !== undefined
          ? defaultOpen
          : streaming === true
            ? true
            : false;
  const [internalOpen, setInternalOpen] = useState(initialOpen);
  const isControlled = controlledOpen !== undefined;
  const open = isControlled ? controlledOpen : internalOpen;

  // Readiness: the horizontal entrance has finished playing. Nested FoldCards
  // cascade in (parent → child). Live cards start ready so a remount does not
  // flash collapsed→open while still streaming. A card that is already open at
  // mount (streaming, or restored from persisted state) must also start ready:
  // otherwise it would mount collapsed, measure short, then pop open 260ms
  // later and jitter the virtual list.
  const [ready, setReady] = useState(streaming === true || internalOpen === true);
  const wantOpen = open && ready;

  // Body grid (`0fr` → `1fr`) needs content mounted to interpolate height.
  // Expand: mount children while still `0fr`, then `is-open` after layout.
  // Collapse: drop `is-open` first, unmount after the row transition ends.
  // Steady collapsed = no descendants, so splitter resize does not layout them.
  const [bodyMounted, setBodyMounted] = useState(wantOpen);
  const [bodyOpen, setBodyOpen] = useState(wantOpen);
  const bodyRef = useRef<HTMLDivElement>(null);
  const wantOpenRef = useRef(wantOpen);
  wantOpenRef.current = wantOpen;

  useLayoutEffect(() => {
    if (wantOpen) {
      if (!bodyMounted) {
        setBodyMounted(true);
        return;
      }
      if (!bodyOpen) setBodyOpen(true);
      return;
    }
    if (bodyOpen) setBodyOpen(false);
  }, [wantOpen, bodyMounted, bodyOpen]);

  useEffect(() => {
    if (wantOpen || bodyOpen || !bodyMounted) return;
    const el = bodyRef.current;
    let ms = 0;
    if (el) {
      const style = getComputedStyle(el);
      ms = cssTimeToMs(style.transitionDuration) + cssTimeToMs(style.transitionDelay);
    }
    if (!(ms > 0)) {
      setBodyMounted(false);
      return;
    }
    const timer = window.setTimeout(() => {
      if (!wantOpenRef.current) setBodyMounted(false);
    }, ms + 50);
    return () => window.clearTimeout(timer);
  }, [wantOpen, bodyOpen, bodyMounted]);

  // Edge blur strengths (single linear value per edge, driven by scroll
  // distance from that edge). 0 = flush with the edge, no band shown.
  const [topStrength, setTopStrength] = useState(0);
  const [bottomStrength, setBottomStrength] = useState(0);

  const contentRef = useRef<HTMLDivElement>(null);
  // Live cards start pinned; sealed cards start unpinned so opening them
  // leaves the reader at the top. User scroll updates this — `streaming`
  // does not force it back on.
  const stickToBottom = useRef(streaming === true);

  // Auto-collapse when streaming ends (process phase done / turn finished).
  // Do NOT auto-reopen on false→true — that caused close→open flicker when the
  // streaming signal briefly dipped between tool batches.
  const prevStreaming = useRef(streaming);
  useLayoutEffect(() => {
    if (prevStreaming.current && !streaming && !isControlled) {
      setInternalOpen(false);
    }
    prevStreaming.current = streaming;
  }, [streaming, isControlled]);

  // Persist the open/collapsed choice on every change so a virtual-list remount
  // (scroll out/in) restores the exact state at unmount — including live cards:
  // a card streaming when it unmounts stays open when it remounts (still live
  // or already sealed), keeping its measured height stable. Skipped for
  // controlled cards and cards without a stable id.
  useEffect(() => {
    if (id === undefined || isControlled) return;
    setFoldCardOpen(id, internalOpen);
  }, [id, isControlled, internalOpen]);

  useEffect(() => {
    if (id === undefined || isControlled) return;
    return subscribeFoldCardOpenRequest((requested) => {
      if (requested !== id) return;
      setReady(true);
      setInternalOpen(true);
    });
  }, [id, isControlled]);

  // Recompute edge blur from current scroll position. Strength is a single
  // linear interpolation of distance-from-edge: 0 at the edge, ramping to
  // BLUR_MAX across BLUR_SCALE px. Only meaningful while overflowing.
  const updateBlur = useCallback(() => {
    const el = contentRef.current;
    if (!el) return;
    const overflow = el.scrollHeight - el.clientHeight > 1;
    if (!overflow) {
      setTopStrength(0);
      setBottomStrength(0);
      return;
    }
    const distTop = el.scrollTop;
    const distBottom = el.scrollHeight - el.scrollTop - el.clientHeight;
    setTopStrength(Math.min(distTop / BLUR_SCALE, 1) * BLUR_MAX);
    setBottomStrength(Math.min(distBottom / BLUR_SCALE, 1) * BLUR_MAX);
  }, []);

  const scrollRef = contentRef;
  const stickToBottomRef = stickToBottom;

  // Re-pin only when streaming flips on (new live window). Do not write
  // `stick = streaming` on every listener bind — that fought user unpin
  // whenever the effect re-attached.
  const prevStreamingForStick = useRef(streaming);
  const prevOpenForStick = useRef(open);
  useEffect(() => {
    const streamingStarted = streaming && !prevStreamingForStick.current;
    const openedWhileLive = open && !prevOpenForStick.current && streaming;
    if (open && (streamingStarted || openedWhileLive)) {
      stickToBottomRef.current = true;
    }
    prevStreamingForStick.current = streaming;
    prevOpenForStick.current = open;
  }, [streaming, open, stickToBottomRef]);

  // Follow the newest line only while the inner scroller is overflowing and
  // the user has not left the bottom. Scroll updates the pin synchronously so
  // a stream flush in the same frame cannot pin them back.
  useEffect(() => {
    const el = scrollRef.current;
    if (!el || !open) return;
    let raf = 0;
    const onScroll = () => {
      stickToBottomRef.current = isNearScrollEnd(el);
      if (raf) return;
      raf = requestAnimationFrame(() => {
        raf = 0;
        updateBlur();
      });
    };
    el.addEventListener("scroll", onScroll, { passive: true });
    updateBlur();
    return () => {
      if (raf) cancelAnimationFrame(raf);
      el.removeEventListener("scroll", onScroll);
    };
  }, [open, updateBlur, scrollRef, stickToBottomRef]);

  useLayoutEffect(() => {
    const el = scrollRef.current;
    if (!el || !open || !stickToBottomRef.current) return;
    if (el.scrollHeight - el.clientHeight <= 1) return;
    el.scrollTop = el.scrollHeight;
    updateBlur();
  }, [children, open, streaming, updateBlur, scrollRef, stickToBottomRef]);

  // Expand only after the entrance has finished (ready). Until then the body
  // stays collapsed and toggle clicks are ignored.
  const canExpand = wantOpen;

  // Returns true when the event originated from an interactive descendant of
  // the header (a button, link, input, etc.) rather than the header surface
  // itself. Tool cards render action buttons (e.g. "Open file") inside the
  // header; a click or key there must drive the button, NOT toggle the card.
  // Walk from the target up to (but not including) the header so we ignore the
  // header's own `role="button"` when the user clicks the bare header surface.
  const originatedFromInteractive = (e: React.SyntheticEvent): boolean => {
    if (e.target === e.currentTarget) return false;
    let node = e.target as HTMLElement | null;
    while (node && node !== e.currentTarget) {
      if (
        node.matches(
          "button, a, input, select, textarea, [role='button'], [contenteditable='true']",
        )
      ) {
        return true;
      }
      node = node.parentElement;
    }
    return false;
  };

  const onClick = (e: React.MouseEvent) => {
    if (originatedFromInteractive(e)) return;
    toggle();
  };

  const toggle = useCallback(() => {
    if (!ready) return;
    const next = !open;
    if (!isControlled) setInternalOpen(next);
    onToggle?.(next);
  }, [ready, open, isControlled, onToggle]);

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key !== "Enter" && e.key !== " ") return;
    if (originatedFromInteractive(e)) return;
    e.preventDefault();
    toggle();
  };

  return (
    <div className={`foldcard min-w-0 max-w-full ${className}`.trim()}>
      <div
        role="button"
        tabIndex={0}
        aria-expanded={canExpand}
        aria-label={headerAriaLabel}
        onClick={onClick}
        onKeyDown={onKeyDown}
        onAnimationEnd={() => setReady(true)}
        className={`foldcard-header flex min-w-0 max-w-full cursor-pointer list-none items-center gap-1.5 select-none py-1 ${
          headerClassName !== ""
            ? headerClassName
            : "text-xs text-(--_dk-text-muted)"
        }`}
      >
        <svg
          width="10"
          height="10"
          viewBox="0 0 10 10"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.5"
          strokeLinecap="round"
          strokeLinejoin="round"
          className={`foldcard-arrow ${FOLDCARD_HEADER_TONE} shrink-0 ${bodyOpen ? "is-open" : ""}`}
        >
          <path d="M3 1.5l4 3.5-4 3.5" />
        </svg>
        {icon != null ? (
          <span className={`${FOLDCARD_HEADER_TONE} inline-flex shrink-0`}>{icon}</span>
        ) : null}
        {typeof label === "string" ? (
          <span className={`${FOLDCARD_HEADER_TONE} min-w-0 flex-1 truncate`}>{label}</span>
        ) : (
          label
        )}
      </div>
      <div
        ref={bodyRef}
        className={`foldcard-body ${bodyOpen ? "is-open" : ""}`}
        onTransitionEnd={(event) => {
          if (event.target !== event.currentTarget) return;
          if (event.propertyName !== "grid-template-rows") return;
          if (!wantOpenRef.current) setBodyMounted(false);
        }}
      >
        <div className="foldcard-body-inner">
          {bodyMounted && (
            <div className="foldcard-scroll-frame">
              <div
                ref={contentRef}
                className={`foldcard-scroll ${contentClassName}`}
              >
                {children}
              </div>
              {topStrength > 0.5 && (
                <ProgressiveBlur
                  side="top"
                  height={BLUR_BAND}
                  strength={topStrength}
                  tint={1}
                  tintCurve={1}
                  tintColor="var(--_dk-editor)"
                />
              )}
              {bottomStrength > 0.5 && (
                <ProgressiveBlur
                  side="bottom"
                  height={BLUR_BAND}
                  strength={bottomStrength}
                  tint={1}
                  tintCurve={1}
                  tintColor="var(--_dk-editor)"
                />
              )}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
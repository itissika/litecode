import { useCallback, useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";

import { ProgressiveBlur } from "./ProgressiveBlur";
import {
  getFoldCardOpenIntent,
  setFoldCardOpenIntent,
  subscribeFoldCardOpenRequest,
  type FoldCardOpenIntent,
} from "./foldCardState";
import { isNearScrollEnd, useStickToBottom } from "../lib/scrollStick";

/** Marks header chrome that should brighten/dim with the FoldCard row. */
export const FOLDCARD_HEADER_TONE = "foldcard-header-tone";

interface FoldCardProps {
  /** Stable identity used to persist open/collapsed across virtual-list remounts.
   *  If omitted, state is purely local (resets on unmount). */
  id?: string;
  /** Controlled open state. If set, parent manages open/close. */
  open?: boolean;
  /** Fallback system state when `autoOpen` is omitted. */
  defaultOpen?: boolean;
  /** Current system open state. Only applies while the user intent is `none`. */
  autoOpen?: boolean;
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
function cssTimeToMs(value: string): number {
  const first = value.split(",")[0]?.trim() ?? "0s";
  if (first.endsWith("ms")) return Number.parseFloat(first) || 0;
  return (Number.parseFloat(first) || 0) * 1000;
}

export function FoldCard({
  id,
  open: controlledOpen,
  defaultOpen,
  autoOpen,
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
  // The persisted state is the user's explicit preference only. If there is no
  // preference, the caller's current system state owns the card on every render.
  const initialIntent = id !== undefined ? getFoldCardOpenIntent(id) : "none";
  const [intent, setIntent] = useState<FoldCardOpenIntent>(initialIntent);
  const isControlled = controlledOpen !== undefined;
  const systemOpen = autoOpen ?? defaultOpen ?? (streaming === true);
  const open = isControlled
    ? controlledOpen
    : intent === "keepopen"
      ? true
      : intent === "keepclosed"
        ? false
        : systemOpen;

  // Readiness: the horizontal entrance has finished playing. Nested FoldCards
  // cascade in (parent → child). Live cards start ready so a remount does not
  // flash collapsed→open while still streaming. A card that is already open at
  // mount (streaming, or restored from persisted state) must also start ready:
  // otherwise it would mount collapsed, measure short, then pop open 260ms
  // later and jitter the virtual list.
  const [ready, setReady] = useState(streaming === true || open === true);
  useLayoutEffect(() => {
    if (open && !ready) setReady(true);
  }, [open, ready]);
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

  // System state owns a card with no explicit user intent. It may open or close
  // on every render; only `keepopen` / `keepclosed` survive a virtual-list remount.
  useEffect(() => {
    if (id === undefined || isControlled) return;
    setFoldCardOpenIntent(id, intent);
  }, [id, isControlled, intent]);

  useEffect(() => {
    if (id === undefined || isControlled) return;
    return subscribeFoldCardOpenRequest((requested) => {
      if (requested !== id) return;
      setReady(true);
      setIntent("keepopen");
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

  // Follow the newest line only while the inner scroller is overflowing and
  // the user has not left the bottom. `stickToBottom` is an authoritative ref
  // driven by gestures (see useStickToBottom): unpin happens synchronously on a
  // scroll-up gesture, so a stream flush in the same frame cannot race ahead
  // and yank the user back down; streaming flips never re-pin an unpinned card.
  const { stickRef: stickToBottom } = useStickToBottom({
    ref: contentRef,
    active: open && bodyMounted,
    initialStick: streaming === true,
    isAtEnd: () => {
      const el = contentRef.current;
      return el ? isNearScrollEnd(el) : false;
    },
    onScroll: updateBlur,
  });

  // Refresh edge blur when the card opens (content may already overflow).
  useEffect(() => {
    updateBlur();
  }, [open, updateBlur]);

  useLayoutEffect(() => {
    const el = contentRef.current;
    if (!el || !open || !stickToBottom.current) return;
    if (el.scrollHeight - el.clientHeight <= 1) return;
    el.scrollTop = el.scrollHeight;
    updateBlur();
  }, [children, open, streaming, updateBlur, stickToBottom]);

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
    if (!isControlled) setIntent(next ? "keepopen" : "keepclosed");
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
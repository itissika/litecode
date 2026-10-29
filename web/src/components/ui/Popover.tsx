import {
  type ReactNode,
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { createPortal } from "react-dom";

import { viewOf } from "../../lib/domView";
import { dockPane, opensUpward, visibleBox, VIEWPORT_MARGIN } from "./dockPane";
import { useFloatingLayer } from "./useFloatingLayer";

interface PopoverApi {
  open: boolean;
  toggle: () => void;
  close: () => void;
}

interface PopoverProps {
  /** How the panel opens: "click" toggles via the trigger's own onClick
   *  (default), "hover" opens after `hoverOpenDelay` and closes once the
   *  pointer leaves both trigger and panel (with a grace period). */
  triggerOn?: "click" | "hover";
  /** Where the floating panel anchors relative to the trigger. Auto-flips
   *  vertically when the requested direction would run off the dock pane (or
   *  the viewport outside dockview). */
  placement?: "up-right" | "up-left" | "down-right" | "down-left";
  /** Fixed pixel width of the panel, or "trigger" to match the trigger width. */
  width?: number | "trigger";
  /** Gap in px between trigger and panel. */
  gap?: number;
  /** Hover mode: delay (ms) before the panel opens. */
  hoverOpenDelay?: number;
  /** Hover mode: grace period (ms) before closing after the pointer leaves
   *  both the trigger and the panel - lets the pointer travel between them. */
  hoverCloseDelay?: number;
  /** Approximate panel height in px, used only to choose the flip direction.
   *  Defaults to the compact panel estimate; tall panels can pass their cap. */
  estimatedHeight?: number;
  /** Classes for the relative wrapper (e.g. "block" for full-width rows). */
  className?: string;
  /** Classes for the panel shell; merged after the default shell. */
  panelClassName?: string;
  /** Trigger renderer. Receives open state + toggle. */
  trigger: (api: PopoverApi) => ReactNode;
  /** Panel content. Receives a close function. */
  children: ReactNode | ((api: { close: () => void }) => ReactNode);
}

const DEFAULT_GAP = 8;
const DEFAULT_WIDTH = 240;
/** Floor for "trigger" width so a degenerate (tiny) trigger still yields a
 *  readable panel. */
const MIN_TRIGGER_WIDTH = 120;
/** Panel height estimate driving the vertical auto-flip. Current panels cap
 *  their scroll area at max-h-48 (192px) plus chrome, so this covers them. */
const PANEL_MAX_H = 260;
/** Smallest pane span that can host a usable panel; below it, close rather
 *  than render a sliver in a collapsed rail. */
const MIN_PANE_SPAN = 48;

/** Fixed-position style object for the portaled panel. */
type PanelPos = {
  top?: number;
  bottom?: number;
  left?: number;
  width?: number;
  maxHeight?: number;
};

function samePos(a: PanelPos | null, b: PanelPos): boolean {
  if (!a) return false;
  return (
    a.top === b.top &&
    a.bottom === b.bottom &&
    a.left === b.left &&
    a.width === b.width &&
    a.maxHeight === b.maxHeight
  );
}

// ---------------------------------------------------------------------------
// Single-open registry: at most one Popover panel may be open app-wide.
// Opening an instance closes the previously open one; closing or unmounting
// unregisters it. Module-level on purpose - no provider needed, and click and
// hover instances enforce the same invariant (bell, ctx ring, commit tips…).
// ---------------------------------------------------------------------------
const openClosers = new Set<() => void>();

function claimSingleOpen(close: () => void): void {
  for (const other of openClosers) {
    if (other !== close) other();
  }
  openClosers.add(close);
}

function releaseSingleOpen(close: () => void): void {
  openClosers.delete(close);
}

/**
 * Portal-based floating panel primitive.
 *
 * Owns only the mechanics: open/close state (click toggle or hover with
 * delays), app-wide single-open exclusion, pane-anchored positioning (fixed
 * + getBoundingClientRect, repositioned on scroll/resize, auto-flip and clamp
 * to the dock pane the trigger lives in; the viewport is the fallback outside
 * dockview) and dismissal (toggle re-click, outside mousedown, Escape). It
 * renders no content of its own - callers inject both
 * the trigger and the panel body, so the same primitive backs notification
 * panels, context-usage popovers, hover commit details, etc.
 */
export function Popover({
  triggerOn = "click",
  placement = "up-right",
  width = DEFAULT_WIDTH,
  gap = DEFAULT_GAP,
  hoverOpenDelay = 150,
  hoverCloseDelay = 150,
  estimatedHeight = PANEL_MAX_H,
  className = "",
  panelClassName = "",
  trigger,
  children,
}: PopoverProps) {
  const [open, setOpen] = useState(false);
  const [pos, setPos] = useState<PanelPos | null>(null);
  const rootRef = useRef<HTMLDivElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  /** The panel's own CSS max-height, measured once per open before the inline
   *  pane cap would mask it. `null` until that first measurement. */
  const styleCapRef = useRef<number | null>(null);
  const openTimer = useRef<number | null>(null);
  const closeTimer = useRef<number | null>(null);
  const hover = triggerOn === "hover";

  const close = useCallback(() => setOpen(false), []);

  const clearTimers = useCallback(() => {
    const view = viewOf(rootRef.current);
    if (openTimer.current !== null) {
      view.clearTimeout(openTimer.current);
      openTimer.current = null;
    }
    if (closeTimer.current !== null) {
      view.clearTimeout(closeTimer.current);
      closeTimer.current = null;
    }
  }, []);

  const update = useCallback(() => {
    const el = rootRef.current;
    if (!el) return;
    const view = viewOf(el);
    const rect = el.getBoundingClientRect();
    const vw = view.innerWidth;
    const vh = view.innerHeight;
    const box = visibleBox(el, vw, vh);
    const pane = dockPane(el);

    // The panel is clamped to its dock pane; once the trigger scrolls out of
    // that pane there is nothing left to anchor to.
    if (pane && (rect.bottom <= box.top || rect.top >= box.bottom)) {
      setOpen(false);
      return;
    }

    const boxW = box.right - box.left;
    const boxH = box.bottom - box.top;
    const minSpan = 2 * VIEWPORT_MARGIN + MIN_PANE_SPAN;
    if (boxW < minSpan || boxH < minSpan) {
      setOpen(false);
      return;
    }

    const wanted =
      width === "trigger" ? Math.max(rect.width, MIN_TRIGGER_WIDTH) : width;
    const w = Math.min(wanted, boxW - 2 * VIEWPORT_MARGIN);

    // Vertical: prefer the requested side, flip when it cannot hold the panel
    // and the other side can (or has more room). The panel is then capped to
    // the room left inside the box.
    const above = Math.max(0, rect.top - box.top);
    const below = Math.max(0, box.bottom - rect.bottom);
    const openUp = opensUpward(
      placement.startsWith("up") ? "up" : "down",
      above,
      below,
      estimatedHeight + gap,
    );
    const room = Math.max(
      0,
      (openUp ? above : below) - gap - VIEWPORT_MARGIN,
    );

    const next: PanelPos = { width: w };
    if (openUp) next.bottom = vh - rect.top + gap;
    else next.top = rect.bottom + gap;

    // Horizontal: anchor to the requested edge, then clamp into the box.
    const anchor = placement.endsWith("left") ? rect.left : rect.right - w;
    next.left = Math.max(
      box.left + VIEWPORT_MARGIN,
      Math.min(anchor, box.right - VIEWPORT_MARGIN - w),
    );

    // Cap the height to the box. The panel's own CSS max-height still wins
    // when it is the tighter limit; it is measured once per open, before this
    // cap is written inline.
    if (styleCapRef.current !== null) {
      next.maxHeight = Math.min(room, styleCapRef.current);
    }

    setPos((prev) => (samePos(prev, next) ? prev : next));
  }, [placement, gap, width, estimatedHeight]);

  useLayoutEffect(() => {
    if (!open) {
      setPos(null);
      styleCapRef.current = null;
      return;
    }
    update();
  }, [open, update]);

  // Measure the panel's own CSS max-height once per open, before writing the
  // pane cap inline (which would otherwise mask it). Then re-run the layout
  // so the cap is min(room inside the pane, the panel's own limit).
  useLayoutEffect(() => {
    if (!open) {
      styleCapRef.current = null;
      return;
    }
    const panel = panelRef.current;
    if (!panel || styleCapRef.current !== null) return;
    const view = viewOf(panel);
    const raw = view.getComputedStyle(panel).maxHeight;
    const px = Number.parseFloat(raw);
    styleCapRef.current =
      raw.endsWith("px") && Number.isFinite(px) ? px : Number.POSITIVE_INFINITY;
    update();
  }, [open, pos, update]);

  // Single-open: claim on open, release on close/unmount.
  useEffect(() => {
    if (!open) return;
    claimSingleOpen(close);
    return () => releaseSingleOpen(close);
  }, [open, close]);

  // Never leave pending hover timers behind on unmount.
  useEffect(() => () => clearTimers(), [clearTimers]);

  useFloatingLayer({
    open,
    rootRef,
    panelRef,
    onDismiss: close,
    // Hover panels are transient: scrolling moves the trigger out from under
    // the pointer (mouseleave is unreliable there), so scroll simply closes.
    onScroll: () => {
      if (hover) {
        clearTimers();
        setOpen(false);
        return;
      }
      update();
    },
    onResize: update,
  });

  // Re-clamp when the dock pane is resized; sash drags do not fire window
  // resize. Same observer as Dropdown.
  useEffect(() => {
    if (!open) return;
    const pane = rootRef.current ? dockPane(rootRef.current) : null;
    if (!pane || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(() => update());
    observer.observe(pane);
    return () => observer.disconnect();
  }, [open, update]);

  // Hover intent: entering the trigger or the panel cancels a pending close;
  // leaving both schedules one. The open side uses a delay so sweeping the
  // pointer across rows does not spam panels.
  const onPointerEnter = useCallback(() => {
    const view = viewOf(rootRef.current);
    if (closeTimer.current !== null) {
      view.clearTimeout(closeTimer.current);
      closeTimer.current = null;
    }
    if (!open && openTimer.current === null) {
      openTimer.current = view.setTimeout(() => {
        openTimer.current = null;
        setOpen(true);
      }, hoverOpenDelay);
    }
  }, [open, hoverOpenDelay]);

  const onPointerLeave = useCallback(() => {
    const view = viewOf(rootRef.current);
    if (openTimer.current !== null) {
      view.clearTimeout(openTimer.current);
      openTimer.current = null;
    }
    if (open && closeTimer.current === null) {
      closeTimer.current = view.setTimeout(() => {
        closeTimer.current = null;
        setOpen(false);
      }, hoverCloseDelay);
    }
  }, [open, hoverCloseDelay]);

  const toggle = useCallback(() => {
    clearTimers();
    setOpen((o) => !o);
  }, [clearTimers]);

  return (
    <div
      ref={rootRef}
      className={className ? `relative ${className}` : "relative inline-flex"}
      onMouseEnter={hover ? onPointerEnter : undefined}
      onMouseLeave={hover ? onPointerLeave : undefined}
    >
      {trigger({ open, toggle, close })}
      {open &&
        pos &&
        createPortal(
          <div
            ref={panelRef}
            data-popover-panel=""
            onMouseEnter={hover ? onPointerEnter : undefined}
            onMouseLeave={hover ? onPointerLeave : undefined}
            className={`fixed z-1000 border border-(--_dk-line-visible) bg-(--_dk-overlay) shadow-[0_6px_18px_rgba(0,0,0,0.18)] ${panelClassName}`}
            style={{
              ...pos,
              overflowY: pos.maxHeight != null ? "auto" : undefined,
            }}
          >
            {typeof children === "function" ? children({ close }) : children}
          </div>,
          rootRef.current?.ownerDocument.body ?? document.body,
        )}
    </div>
  );
}

import { type ReactNode, useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";

export interface PopoverApi {
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
   *  vertically when the requested direction would run off the viewport. */
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
/** Minimum distance kept between the panel and the viewport edges. */
const VIEWPORT_MARGIN = 8;

/** Fixed-position style object for the portaled panel. */
type PanelPos = {
  top?: number;
  right?: number;
  bottom?: number;
  left?: number;
  width?: number;
};

function samePos(a: PanelPos | null, b: PanelPos): boolean {
  if (!a) return false;
  return (
    a.top === b.top &&
    a.right === b.right &&
    a.bottom === b.bottom &&
    a.left === b.left &&
    a.width === b.width
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
 * delays), app-wide single-open exclusion, viewport-anchored positioning
 * (fixed + getBoundingClientRect, repositioned on scroll/resize, auto-flip
 * and clamp at the viewport edges) and dismissal (toggle re-click, outside
 * mousedown, Escape). It renders no content of its own - callers inject both
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
  className = "",
  panelClassName = "",
  trigger,
  children,
}: PopoverProps) {
  const [open, setOpen] = useState(false);
  const [pos, setPos] = useState<PanelPos | null>(null);
  const rootRef = useRef<HTMLDivElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const openTimer = useRef<number | null>(null);
  const closeTimer = useRef<number | null>(null);
  const hover = triggerOn === "hover";

  const close = useCallback(() => setOpen(false), []);

  const clearTimers = useCallback(() => {
    if (openTimer.current !== null) {
      window.clearTimeout(openTimer.current);
      openTimer.current = null;
    }
    if (closeTimer.current !== null) {
      window.clearTimeout(closeTimer.current);
      closeTimer.current = null;
    }
  }, []);

  const update = useCallback(() => {
    const el = rootRef.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    const w =
      width === "trigger" ? Math.max(rect.width, MIN_TRIGGER_WIDTH) : width;

    const next: PanelPos = { width: w };

    // Vertical: honor the requested direction, flip when the estimated panel
    // would run off the viewport (same policy as Dropdown).
    const fitsDown = vh - rect.bottom >= PANEL_MAX_H + VIEWPORT_MARGIN;
    const fitsUp = rect.top >= PANEL_MAX_H + VIEWPORT_MARGIN;
    if (placement.startsWith("up")) {
      if (fitsUp) next.bottom = vh - rect.top + gap;
      else next.top = rect.bottom + gap; // flip down
    } else {
      if (fitsDown) next.top = rect.bottom + gap;
      else next.bottom = vh - rect.top + gap; // flip up
    }

    // Horizontal: anchor to the requested edge, clamped into the viewport.
    if (placement.endsWith("left")) {
      next.left = Math.max(
        VIEWPORT_MARGIN,
        Math.min(rect.left, vw - w - VIEWPORT_MARGIN),
      );
    } else {
      next.right = Math.max(
        VIEWPORT_MARGIN,
        Math.min(vw - rect.right, vw - w - VIEWPORT_MARGIN),
      );
    }

    setPos((prev) => (samePos(prev, next) ? prev : next));
  }, [placement, gap, width]);

  useLayoutEffect(() => {
    if (!open) {
      setPos(null);
      return;
    }
    update();
  }, [open, update]);

  // Single-open: claim on open, release on close/unmount.
  useEffect(() => {
    if (!open) return;
    claimSingleOpen(close);
    return () => releaseSingleOpen(close);
  }, [open, close]);

  // Never leave pending hover timers behind on unmount.
  useEffect(() => () => clearTimers(), [clearTimers]);

  useEffect(() => {
    if (!open) return;
    // Hover panels are transient: scrolling moves the trigger out from under
    // the pointer (mouseleave is unreliable there), so scroll simply closes.
    const onScroll = () => {
      if (hover) {
        clearTimers();
        setOpen(false);
        return;
      }
      update();
    };
    const onResize = () => update();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    const onDown = (e: MouseEvent) => {
      const t = e.target as Node;
      if (rootRef.current?.contains(t)) return;
      if (panelRef.current?.contains(t)) return;
      setOpen(false);
    };
    window.addEventListener("scroll", onScroll, true);
    window.addEventListener("resize", onResize);
    window.addEventListener("keydown", onKey);
    document.addEventListener("mousedown", onDown);
    return () => {
      window.removeEventListener("scroll", onScroll, true);
      window.removeEventListener("resize", onResize);
      window.removeEventListener("keydown", onKey);
      document.removeEventListener("mousedown", onDown);
    };
  }, [open, hover, update, clearTimers]);

  // Hover intent: entering the trigger or the panel cancels a pending close;
  // leaving both schedules one. The open side uses a delay so sweeping the
  // pointer across rows does not spam panels.
  const onPointerEnter = useCallback(() => {
    if (closeTimer.current !== null) {
      window.clearTimeout(closeTimer.current);
      closeTimer.current = null;
    }
    if (!open && openTimer.current === null) {
      openTimer.current = window.setTimeout(() => {
        openTimer.current = null;
        setOpen(true);
      }, hoverOpenDelay);
    }
  }, [open, hoverOpenDelay]);

  const onPointerLeave = useCallback(() => {
    if (openTimer.current !== null) {
      window.clearTimeout(openTimer.current);
      openTimer.current = null;
    }
    if (open && closeTimer.current === null) {
      closeTimer.current = window.setTimeout(() => {
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
            onMouseEnter={hover ? onPointerEnter : undefined}
            onMouseLeave={hover ? onPointerLeave : undefined}
            className={`fixed z-1000 border border-(--_dk-line-visible) bg-(--_dk-overlay) shadow-[0_6px_18px_rgba(0,0,0,0.18)] ${panelClassName}`}
            style={pos}
          >
            {typeof children === "function" ? children({ close }) : children}
          </div>,
          document.body,
        )}
    </div>
  );
}

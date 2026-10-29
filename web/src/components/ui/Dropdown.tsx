import {
  type ReactNode,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { createPortal } from "react-dom";

import { dockPane, opensUpward, visibleBox, VIEWPORT_MARGIN } from "./dockPane";
import { useFloatingLayer } from "./useFloatingLayer";

/** Display + width fallback per variant, applied to the root wrapper.
 *  Placed BEFORE the caller's `className`, so any display/width utility the
 *  caller passes overrides it (equal specificity, source order wins). The
 *  fallback is content-adaptive (`inline-flex w-auto`) for every variant — the
 *  root shrinks to its trigger's content instead of forcing a width. Callers
 *  that need to fill a column pass `w-full` themselves (e.g. form Selects). */
const VARIANT_SHELL: Record<DropdownVariant, string> = {
  select: "inline-flex w-auto",
  menu: "inline-flex w-auto",
  panel: "inline-flex w-auto",
};

/** Visual presets for the panel shell.
 *  Borderless, no vertical padding, hugs the trigger (no gap). Background and
 *  shadow are applied separately (see DEFAULT_BG / SHADOW) so they can be
 *  overridden per-instance without fighting the cascade. */
const VARIANT_PANEL: Record<DropdownVariant, string> = {
  select: "w-max max-w-[360px] max-h-48 overflow-y-auto",
  menu: "min-w-[160px] overflow-y-auto",
  panel: "overflow-y-auto",
};

/** Select menus stay at `max-h-48` unless a caller asks for something else.
 *  Still clamped to the open side of the screen and the pane. */
const VARIANT_MAX_H: Record<DropdownVariant, number | undefined> = {
  select: 192,
  menu: undefined,
  panel: undefined,
};

/** Default panel background. Overridable via the `bgClassName` prop. */
const DEFAULT_BG = "bg-(--_dk-overlay)";

/** Soft shadow with offset == blur radius: the edge hugging the trigger is
 *  genuinely clean (blur fades to nothing exactly at the panel edge), while the
 *  rest of the panel still gets a gentle, low-opacity lift. */
const SHADOW: Record<"up" | "down", string> = {
  down: "shadow-[0_6px_18px_rgba(0,0,0,0.18)]",
  up: "shadow-[0_6px_18px_rgba(0,0,0,0.18)]",
};

/** Same edge as Popover (bell / context-usage panels). */
const BORDER: Record<"up" | "down", string> = {
  down: "border border-(--_dk-line-visible)",
  up: "border border-(--_dk-line-visible)",
};

type DropdownVariant = "select" | "menu" | "panel";

/** Shared item classes for select/menu variants — import and apply per item
 *  so item styling is also defined in one place. */
export const dropdownItemClass =
  "block w-full whitespace-nowrap overflow-hidden text-ellipsis px-3 py-1.5 text-left text-[11px] text-(--_dk-text-primary) hover:bg-(--_dk-ix-bg-hover) cursor-pointer";
export const dropdownItemActiveClass = "text-(--_dk-ix-fg-selected)";

/** Panel max width per variant — mirrors the CSS `max-w` on the select shell.
 *  Used to clamp the portaled panel inside the viewport. */
const PANEL_MAX_W: Record<DropdownVariant, number> = {
  select: 360,
  menu: 240,
  panel: 320,
};

/** Fixed-position style object for the portaled panel. */
type PanelPos = {
  top?: number;
  bottom?: number;
  left?: number;
  right?: number;
  width?: number;
  minWidth?: number;
  maxWidth?: number;
  maxHeight?: number;
};

function samePos(a: PanelPos | null, b: PanelPos): boolean {
  if (!a) return false;
  return (
    a.top === b.top &&
    a.bottom === b.bottom &&
    a.left === b.left &&
    a.right === b.right &&
    a.width === b.width &&
    a.minWidth === b.minWidth &&
    a.maxWidth === b.maxWidth &&
    a.maxHeight === b.maxHeight
  );
}

interface DropdownProps {
  /** Direction the panel prefers. It still opens the other way — down
   *  included — when that side of the screen (and the current pane) has
   *  the room. */
  direction?: "up" | "down";
  /** When set, the flip decision uses the panel's measured height instead of
   *  only `maxHeight`. The menu still stays inside the screen and the pane. */
  flip?: boolean;
  /**
   * Horizontal alignment of the panel to the wrapper.
   * - "left" / "right": anchor to that edge (default "left")
   * - "stretch": span the full wrapper width
   * - "none": treated as "left" (the panel is portaled to the body, so
   *   container-relative insets no longer apply)
   */
  align?: "left" | "right" | "stretch" | "none";
  /** Panel visual preset. Drives the default shell styling. */
  variant?: DropdownVariant;
  /** Classes for the relative wrapper (layout: shrink-0, w-full, …). */
  className?: string;
  /** Extra panel classes, merged after the variant's default shell. */
  panelClassName?: string;
  /** Panel background. Overrides the default `bg-(--_dk-overlay)` (e.g. to
   *  match the trigger for a seamless, borderless look). */
  bgClassName?: string;
  /** Auto-close when a click occurs inside the panel (default true unless variant="panel"). */
  closeOnSelect?: boolean;
  /** Preferred max height in px. Kept when the side the menu opens toward
   *  has at least this much room; otherwise it is dropped and the menu is
   *  capped to that side of the screen, clipped to the current pane.
   *  `null` skips the variant ceiling (select's 192px) and uses the free
   *  space. Omit to keep the variant ceiling, still clamped to that space. */
  maxHeight?: number | null;
  /** Trigger renderer. Receives the open state and a toggle function. */
  trigger: (api: { open: boolean; toggle: () => void }) => ReactNode;
  /** Panel content. `maxHeight` is the cap actually applied (the wish, or
   *  the free space when the wish does not fit). `placement` is the side
   *  the menu opened toward. */
  children:
    | ReactNode
    | ((api: {
        close: () => void;
        maxHeight: number;
        placement: "up" | "down";
      }) => ReactNode);
}

/**
 * Unified dropdown primitive.
 *
 * Owns: open/close state, outside-click (mousedown) dismissal, Escape
 * dismissal, and viewport-anchored positioning (fixed + getBoundingClientRect,
 * repositioned on scroll/resize — same mechanics as Popover). The panel opens
 * downward when that side has the room, and its width and height are clamped
 * to the screen; height is also clamped to the dock pane the trigger lives
 * in. A caller may pass `maxHeight` as a wish — it is kept only when that
 * side can hold it. The panel is rendered through a portal to the trigger's
 * document body, so it escapes any ancestor overflow/stacking context (fold
 * cards, scroll containers, dialogs) instead of being clipped by them. In the
 * main window that body is `document.body`. The panel's shell
 * styling comes from `variant`, so the look is edited in exactly one place.
 * Callers supply only the trigger button and the panel content.
 *
 * Distinct from FloatingDialog (a draggable/resizable modal window).
 */
export function Dropdown({
  direction = "down",
  flip = false,
  align = "left",
  variant = "select",
  className = "",
  panelClassName = "",
  bgClassName,
  closeOnSelect,
  maxHeight,
  trigger,
  children,
}: DropdownProps) {
  const [open, setOpen] = useState(false);
  const [pos, setPos] = useState<PanelPos | null>(null);
  const rootRef = useRef<HTMLDivElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  /** Measured portaled-panel height. `flip` uses it as the wish when the
   *  caller did not pass `maxHeight`. */
  const panelHRef = useRef<number | null>(null);

  const autoClose = closeOnSelect ?? variant !== "panel";

  const update = () => {
    const el = rootRef.current;
    if (!el) return;
    const view = el.ownerDocument.defaultView ?? window;
    const rect = el.getBoundingClientRect();
    const vw = view.innerWidth;
    const vh = view.innerHeight;
    const box = visibleBox(el, vw, vh);
    const above = Math.max(0, rect.top - box.top);
    const below = Math.max(0, box.bottom - rect.bottom);
    // `null` is an explicit "no ceiling of my own". `undefined` keeps the
    // variant's ceiling. A finite wish is kept only when the open side holds it.
    const wish =
      maxHeight === null
        ? undefined
        : (maxHeight ??
          (flip ? (panelHRef.current ?? undefined) : undefined) ??
          VARIANT_MAX_H[variant]);
    const opensUp = opensUpward(direction, above, below, wish);
    const room = Math.max(0, (opensUp ? above : below) - VIEWPORT_MARGIN);
    const cap =
      wish != null && Number.isFinite(wish)
        ? Math.min(Math.max(0, wish), room)
        : room;

    const next: PanelPos = { maxHeight: cap };
    if (opensUp) next.bottom = vh - rect.top;
    else next.top = rect.bottom;

    // Width is reserved up to the variant's max, then shifted and capped so
    // the panel stays inside the viewport. A narrower panel simply doesn't
    // fill that reservation.
    const span = Math.max(
      0,
      Math.min(PANEL_MAX_W[variant], vw - 2 * VIEWPORT_MARGIN),
    );
    if (align === "stretch") {
      const width = Math.min(rect.width, Math.max(0, vw - 2 * VIEWPORT_MARGIN));
      let left = rect.left;
      if (left + width > vw - VIEWPORT_MARGIN) left = vw - VIEWPORT_MARGIN - width;
      if (left < VIEWPORT_MARGIN) left = VIEWPORT_MARGIN;
      next.left = left;
      next.width = width;
      next.maxWidth = width;
    } else if (align === "right") {
      let panelRight = Math.min(rect.right, vw - VIEWPORT_MARGIN);
      if (panelRight - span < VIEWPORT_MARGIN) {
        panelRight = Math.min(vw - VIEWPORT_MARGIN, VIEWPORT_MARGIN + span);
      }
      next.right = vw - panelRight;
      next.maxWidth = Math.min(span, Math.max(0, panelRight - VIEWPORT_MARGIN));
    } else {
      let left = rect.left;
      if (left + span > vw - VIEWPORT_MARGIN) {
        left = vw - VIEWPORT_MARGIN - span;
      }
      if (left < VIEWPORT_MARGIN) left = VIEWPORT_MARGIN;
      next.left = left;
      next.maxWidth = Math.min(span, Math.max(0, vw - VIEWPORT_MARGIN - left));
    }

    if (variant === "select") {
      // Replaces the old absolute `min-w-full`: the panel is at least as wide
      // as the trigger. With fixed positioning `min-width: 100%` would resolve
      // against the viewport, so the trigger width is passed in px instead.
      // Never wider than the clamp, or the min would push the panel back out.
      next.minWidth = Math.min(rect.width, next.maxWidth ?? rect.width);
    }

    setPos((prev) => (samePos(prev, next) ? prev : next));
  };

  useLayoutEffect(() => {
    if (!open) {
      setPos(null);
      panelHRef.current = null;
      return;
    }
    update();
  }, [open, direction, align, variant, maxHeight, flip]);

  // `flip` re-decides once the real height is known. The equality guard on
  // `panelHRef` keeps a capped panel from looping.
  useLayoutEffect(() => {
    if (!open || !flip || !pos) return;
    const el = panelRef.current;
    if (!el) return;
    const h = el.offsetHeight;
    if (h > 0 && h !== panelHRef.current) {
      panelHRef.current = h;
      update();
    }
  }, [open, flip, pos, maxHeight]);

  useFloatingLayer({
    open,
    rootRef,
    panelRef,
    onDismiss: () => setOpen(false),
    onScroll: update,
    onResize: update,
  });

  // Re-clamp when the dock pane the trigger lives in is resized.
  useEffect(() => {
    if (!open) return;
    const pane = rootRef.current ? dockPane(rootRef.current) : null;
    if (!pane || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(() => update());
    observer.observe(pane);
    return () => observer.disconnect();
  }, [open, direction, align, variant, maxHeight, flip]);

  const placement: "up" | "down" = pos?.bottom != null ? "up" : "down";

  return (
    <div
      ref={rootRef}
      className={`relative ${VARIANT_SHELL[variant]} ${className}`}
    >
      {trigger({ open, toggle: () => setOpen((o) => !o) })}
      {open &&
        pos &&
        createPortal(
          <div
            ref={panelRef}
            data-dropdown-panel
            className={`fixed z-[10000] overflow-x-hidden ${SHADOW[placement]} ${BORDER[placement]} ${bgClassName ?? DEFAULT_BG} ${VARIANT_PANEL[variant]} ${panelClassName}`}
            style={pos}
            onClick={autoClose ? () => setOpen(false) : undefined}
          >
            {typeof children === "function"
              ? children({
                  close: () => setOpen(false),
                  maxHeight: pos.maxHeight ?? 0,
                  placement,
                })
              : children}
          </div>,
          rootRef.current?.ownerDocument.body ?? document.body,
        )}
    </div>
  );
}

import {
  type ReactNode,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { createPortal } from "react-dom";

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

export type DropdownVariant = "select" | "menu" | "panel";

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

/** Keep the portaled panel at least this far from the viewport edges. */
const VIEWPORT_MARGIN = 8;

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

type SideBox = { top: number; right: number; bottom: number; left: number };

/** Box the menu must stay inside. Dockview lifts panel content out of
 *  `.dv-groupview` into `.dv-render-overlay` and positions that overlay on
 *  the group's content box, so the trigger has no groupview ancestor.
 *  Walking up to the group misses the pane and the menu only meets the
 *  screen. The overlay's rect is the absolute top and bottom of the pane.
 *  A groupview ancestor is the fallback for content that was not relocated. */
function dockPane(el: HTMLElement): HTMLElement | null {
  const overlay = el.closest<HTMLElement>(".dv-render-overlay");
  if (overlay) {
    const rect = overlay.getBoundingClientRect();
    if (rect.width > 0 && rect.height > 0) return overlay;
  }
  const group = el.closest<HTMLElement>(".dv-groupview");
  if (!group) return null;
  for (const child of group.children) {
    if (child.classList.contains("dv-content-container")) {
      return child as HTMLElement;
    }
  }
  return group;
}

/** Viewport clipped to the pane. Height uses this box; a pane that hangs
 *  off the screen does not lend the menu that off-screen space. */
function visibleBox(el: HTMLElement, vw: number, vh: number): SideBox {
  const pane = dockPane(el)?.getBoundingClientRect();
  if (!pane) return { top: 0, right: vw, bottom: vh, left: 0 };
  return {
    top: Math.max(0, pane.top),
    left: Math.max(0, pane.left),
    right: Math.min(vw, pane.right),
    bottom: Math.min(vh, pane.bottom),
  };
}

/** Prefer `direction`. A finite wish keeps that side when the side can hold
 *  it; otherwise the menu opens toward whichever side of the box is taller
 *  (down included). */
function opensUpward(
  direction: "up" | "down",
  above: number,
  below: number,
  wish: number | undefined,
): boolean {
  const holds = (room: number) =>
    wish != null && room >= wish + VIEWPORT_MARGIN;
  if (direction === "up") {
    if (holds(above)) return true;
    if (holds(below)) return false;
    return above >= below;
  }
  if (holds(below)) return false;
  if (holds(above)) return true;
  return above > below;
}

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
 * side can hold it. The panel is rendered through a portal to `document.body`,
 * so it escapes any ancestor overflow/stacking context (fold cards, scroll
 * containers, dialogs) instead of being clipped by them. The panel's shell
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
    const rect = el.getBoundingClientRect();
    const vw = window.innerWidth;
    const vh = window.innerHeight;
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

  useEffect(() => {
    if (!open) return;
    const onScroll = () => update();
    const onResize = () => update();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    const onDown = (e: MouseEvent) => {
      const t = e.target as Node;
      if (rootRef.current && rootRef.current.contains(t)) return;
      if (panelRef.current && panelRef.current.contains(t)) return;
      setOpen(false);
    };
    window.addEventListener("scroll", onScroll, true);
    window.addEventListener("resize", onResize);
    window.addEventListener("keydown", onKey);
    document.addEventListener("mousedown", onDown);
    const pane = rootRef.current ? dockPane(rootRef.current) : null;
    let observer: ResizeObserver | null = null;
    if (pane && typeof ResizeObserver !== "undefined") {
      observer = new ResizeObserver(() => update());
      observer.observe(pane);
    }
    return () => {
      observer?.disconnect();
      window.removeEventListener("scroll", onScroll, true);
      window.removeEventListener("resize", onResize);
      window.removeEventListener("keydown", onKey);
      document.removeEventListener("mousedown", onDown);
    };
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
          document.body,
        )}
    </div>
  );
}

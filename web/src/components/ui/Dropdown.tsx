import { type ReactNode, useEffect, useLayoutEffect, useRef, useState } from "react";
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
  menu: "min-w-[160px]",
  panel: "",
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

/** Panel max height (select shell's `max-h-48`). Used to auto-flip the panel
 *  when the requested direction would run off the viewport. */
const PANEL_MAX_H = 192;

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
};

interface DropdownProps {
  /** Direction the panel opens relative to the trigger. Auto-flips when the
   *  requested direction would run off the viewport. */
  direction?: "up" | "down";
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
  /** Trigger renderer. Receives the open state and a toggle function. */
  trigger: (api: { open: boolean; toggle: () => void }) => ReactNode;
  /** Panel content. Receives a close function. */
  children: ReactNode | ((api: { close: () => void }) => ReactNode);
}

/**
 * Unified dropdown primitive.
 *
 * Owns: open/close state, outside-click (mousedown) dismissal, Escape
 * dismissal, and viewport-anchored positioning (fixed + getBoundingClientRect,
 * repositioned on scroll/resize — same mechanics as Popover). The panel is
 * rendered through a portal to `document.body`, so it escapes any ancestor
 * overflow/stacking context (fold cards, scroll containers, dialogs) instead
 * of being clipped by them. The panel's shell styling comes from `variant`,
 * so the look is edited in exactly one place. Callers supply only the trigger
 * button and the panel content.
 *
 * Distinct from FloatingDialog (a draggable/resizable modal window).
 */
export function Dropdown({
  direction = "down",
  align = "left",
  variant = "select",
  className = "",
  panelClassName = "",
  bgClassName,
  closeOnSelect,
  trigger,
  children,
}: DropdownProps) {
  const [open, setOpen] = useState(false);
  const [pos, setPos] = useState<PanelPos | null>(null);
  const rootRef = useRef<HTMLDivElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);

  const autoClose = closeOnSelect ?? variant !== "panel";

  const update = () => {
    const el = rootRef.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    const maxW = PANEL_MAX_W[variant];

    const next: PanelPos = {};

    // Vertical: honor the requested direction, but flip when the panel would
    // run off the viewport — the portaled panel is no longer clipped by any
    // container, so the viewport edge is the only boundary left.
    const spaceBelow = vh - rect.bottom;
    const spaceAbove = rect.top;
    const fitsDown = spaceBelow >= PANEL_MAX_H + VIEWPORT_MARGIN;
    const fitsUp = spaceAbove >= PANEL_MAX_H + VIEWPORT_MARGIN;
    if (direction === "up") {
      if (fitsUp) next.bottom = vh - rect.top;
      else next.top = rect.bottom; // flip down
    } else {
      if (fitsDown) next.top = rect.bottom;
      else next.bottom = vh - rect.top; // flip up
    }

    // Horizontal anchoring + viewport clamp.
    if (align === "stretch") {
      next.left = rect.left;
      next.width = rect.width;
    } else if (align === "right") {
      next.right = vw - rect.right;
    } else {
      // left (default) and none → left-anchored, clamped into the viewport.
      next.left = Math.max(
        VIEWPORT_MARGIN,
        Math.min(rect.left, vw - maxW - VIEWPORT_MARGIN),
      );
    }

    if (variant === "select") {
      // Replaces the old absolute `min-w-full`: the panel is at least as wide
      // as the trigger. With fixed positioning `min-width: 100%` would resolve
      // against the viewport, so the trigger width is passed in px instead.
      next.minWidth = rect.width;
    }

    setPos(next);
  };

  useLayoutEffect(() => {
    if (!open) {
      setPos(null);
      return;
    }
    update();
  }, [open, direction, align, variant]);

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
    return () => {
      window.removeEventListener("scroll", onScroll, true);
      window.removeEventListener("resize", onResize);
      window.removeEventListener("keydown", onKey);
      document.removeEventListener("mousedown", onDown);
    };
  }, [open]);

  return (
    <div ref={rootRef} className={`relative ${VARIANT_SHELL[variant]} ${className}`}>
      {trigger({ open, toggle: () => setOpen((o) => !o) })}
      {open &&
        pos &&
        createPortal(
          <div
            ref={panelRef}
            className={`fixed z-[10000] ${SHADOW[direction]} ${BORDER[direction]} ${bgClassName ?? DEFAULT_BG} ${VARIANT_PANEL[variant]} ${panelClassName}`}
            style={pos}
            onClick={autoClose ? () => setOpen(false) : undefined}
          >
            {typeof children === "function"
              ? children({ close: () => setOpen(false) })
              : children}
          </div>,
          document.body,
        )}
    </div>
  );
}
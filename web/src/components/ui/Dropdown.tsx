import { type ReactNode, useEffect, useRef, useState } from "react";

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
  select: "min-w-full w-max max-w-[360px] max-h-48 overflow-y-auto",
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

interface DropdownProps {
  /** Direction the panel opens relative to the trigger. */
  direction?: "up" | "down";
  /**
   * Horizontal alignment of the panel to the wrapper.
   * - "left" / "right": anchor to that edge (default "left")
   * - "stretch": span the full wrapper width
   * - "none": let panelClassName position it (e.g. insets like left-3 right-3)
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
 * dismissal, and directional positioning (up/down + alignment). The panel's
 * shell styling comes from `variant`, so the look is edited in exactly one
 * place. Callers supply only the trigger button and the panel content.
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
  const rootRef = useRef<HTMLDivElement>(null);

  const autoClose = closeOnSelect ?? variant !== "panel";

  useEffect(() => {
    if (!open) return;
    const onMouseDown = (e: MouseEvent) => {
      if (rootRef.current && !rootRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", onMouseDown);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("mousedown", onMouseDown);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [open]);

  // Hugs the trigger: no margin gap.
  const posCls = direction === "up" ? "bottom-full" : "top-full";
  const alignCls =
    align === "right"
      ? "right-0"
      : align === "stretch"
        ? "left-0 right-0"
        : align === "none"
          ? ""
          : "left-0";

  return (
    <div ref={rootRef} className={`relative ${VARIANT_SHELL[variant]} ${className}`}>
      {trigger({ open, toggle: () => setOpen((o) => !o) })}
      {open && (
        <div
          className={`absolute z-50 ${posCls} ${alignCls} ${SHADOW[direction]} ${BORDER[direction]} ${bgClassName ?? DEFAULT_BG} ${VARIANT_PANEL[variant]} ${panelClassName}`}
          onClick={autoClose ? () => setOpen(false) : undefined}
        >
          {typeof children === "function"
            ? children({ close: () => setOpen(false) })
            : children}
        </div>
      )}
    </div>
  );
}

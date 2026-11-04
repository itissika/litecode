import { createPortal } from "react-dom";
import { useEffect, useLayoutEffect, useRef, useState } from "react";

import { viewOf } from "../../lib/domView";
import { visibleBox, VIEWPORT_MARGIN } from "./dockPane";
import { useFloatingLayer } from "./useFloatingLayer";
import { usePortalTarget } from "./usePortalTarget";

export interface ContextMenuItem {
  id: string;
  label?: string;
  shortcut?: string;
  disabled?: boolean;
  danger?: boolean;
  separator?: boolean;
  /** Return focus to the trigger after the action. Defaults to true; an
   *  action that opens an inline editor or dialog owns focus itself. */
  restoreFocus?: boolean;
  onClick?: () => void;
}

interface ContextMenuProps {
  x: number;
  y: number;
  items: ContextMenuItem[];
  onClose: () => void;
  ariaLabel?: string;
}

/**
 * Cursor-anchored menu shared by every row/grid surface.
 *
 * Owns the mechanics a context menu must not re-implement: portal into the
 * trigger's document, pane/viewport clamping, scroll/resize dismissal,
 * outside-click dismissal, roving focus, Escape/Tab close, and focus return.
 */
export function ContextMenu({
  x,
  y,
  items,
  onClose,
  ariaLabel = "Context menu",
}: ContextMenuProps) {
  const { anchorRef, target } = usePortalTarget<HTMLSpanElement>();
  const panelRef = useRef<HTMLDivElement>(null);
  const closeRef = useRef(onClose);
  closeRef.current = onClose;
  const restoreFocusRef = useRef(true);
  const activeElementRef = useRef<HTMLElement | null>(null);
  /** Element under the cursor before the portal paints. A scroll in its
   *  ancestor scroller moves the menu's anchor; unrelated scrollers (the
   *  streaming transcript) must leave the menu alone. */
  const sourceNodeRef = useRef<Node | null>(null);
  const focusedOnceRef = useRef(false);
  const [pos, setPos] = useState<{
    left: number;
    top: number;
    maxWidth?: number;
    maxHeight?: number;
  }>({ left: x, top: y });

  useLayoutEffect(() => {
    const anchor = anchorRef.current;
    const doc = anchor?.ownerDocument ?? document;
    const active = doc.activeElement;
    activeElementRef.current =
      active && typeof (active as HTMLElement).focus === "function"
        ? (active as HTMLElement)
        : null;
    if (!sourceNodeRef.current) {
      const at =
        typeof doc.elementFromPoint === "function"
          ? doc.elementFromPoint(x, y)
          : null;
      sourceNodeRef.current = at ?? activeElementRef.current;
    }
  }, [anchorRef, x, y]);

  useLayoutEffect(() => {
    if (!target) return;
    const panel = panelRef.current;
    if (!panel) return;
    const view = viewOf(panel);
    const box = visibleBox(panel, view.innerWidth, view.innerHeight);
    const maxWidth = Math.max(0, box.right - box.left - VIEWPORT_MARGIN * 2);
    const maxHeight = Math.max(0, box.bottom - box.top - VIEWPORT_MARGIN * 2);

    if (pos.maxHeight !== maxHeight || pos.maxWidth !== maxWidth) {
      setPos({ left: x, top: y, maxWidth, maxHeight });
      return;
    }

    const rect = panel.getBoundingClientRect();
    const left = Math.max(
      box.left + VIEWPORT_MARGIN,
      Math.min(x, box.right - rect.width - VIEWPORT_MARGIN),
    );
    const top = Math.max(
      box.top + VIEWPORT_MARGIN,
      Math.min(y, box.bottom - rect.height - VIEWPORT_MARGIN),
    );
    setPos((current) =>
      current.left === left && current.top === top
        ? current
        : { ...current, left, top },
    );
    if (!focusedOnceRef.current) {
      focusedOnceRef.current = true;
      const first = panel.querySelector<HTMLElement>(
        'button[role="menuitem"]:not([disabled])',
      );
      (first ?? panel).focus();
    }
  }, [target, x, y, items.length, pos.maxWidth, pos.maxHeight]);

  useEffect(() => {
    return () => {
      if (!restoreFocusRef.current) return;
      const active = activeElementRef.current;
      if (active?.isConnected) active.focus();
    };
  }, []);

  const closeWithoutRestore = () => {
    restoreFocusRef.current = false;
    closeRef.current();
  };
  const closeWithRestore = () => {
    restoreFocusRef.current = true;
    closeRef.current();
  };

  useFloatingLayer({
    open: target !== null,
    rootRef: anchorRef,
    panelRef,
    onDismiss: closeWithoutRestore,
    onEscape: closeWithRestore,
    // A streaming transcript scrolls its own scroller while the menu is open.
    // Close only when the scrolled element actually owns this menu's anchor;
    // unrelated scrollers must not dismiss it.
    onScroll: (event) => {
      const anchor = anchorRef.current;
      const target = event.target as Node | null;
      const source = sourceNodeRef.current;
      if (!anchor || !target) return;
      if (typeof target.contains !== "function") return;
      if (
        target === anchor.ownerDocument ||
        (source && target.contains(source)) ||
        target.contains(anchor)
      ) {
        closeWithoutRestore();
      }
    },
    onResize: closeWithoutRestore,
  });

  const moveFocus = (delta: number | "first" | "last") => {
    const panel = panelRef.current;
    if (!panel) return;
    const buttons = [
      ...panel.querySelectorAll<HTMLElement>(
        'button[role="menuitem"]:not([disabled])',
      ),
    ];
    if (buttons.length === 0) return;
    const current = buttons.indexOf(document.activeElement as HTMLElement);
    let next = 0;
    if (delta === "first") next = 0;
    else if (delta === "last") next = buttons.length - 1;
    else if (current < 0) next = delta > 0 ? 0 : buttons.length - 1;
    else next = (current + delta + buttons.length) % buttons.length;
    buttons[next]?.focus();
  };

  return (
    <>
      <span ref={anchorRef} hidden aria-hidden />
      {target
        ? createPortal(
            <div
              ref={panelRef}
              role="menu"
              aria-label={ariaLabel}
              tabIndex={-1}
              data-litecode-overlay="context-menu"
              className="fixed z-(--_dk-z-menu) min-w-[198px] overflow-y-auto rounded border border-(--_dk-line) bg-(--_dk-editor) py-1 text-sm shadow-(--_dk-shadow-menu)"
              style={{
                left: pos.left,
                top: pos.top,
                maxWidth: pos.maxWidth,
                maxHeight: pos.maxHeight,
              }}
              onContextMenu={(event) => event.preventDefault()}
              onKeyDown={(event) => {
                if (event.key === "ArrowDown") {
                  event.preventDefault();
                  moveFocus(1);
                } else if (event.key === "ArrowUp") {
                  event.preventDefault();
                  moveFocus(-1);
                } else if (event.key === "Home") {
                  event.preventDefault();
                  moveFocus("first");
                } else if (event.key === "End") {
                  event.preventDefault();
                  moveFocus("last");
                } else if (event.key === "Tab") {
                  event.preventDefault();
                  closeWithRestore();
                }
              }}
            >
              {items.map((item) =>
                item.separator ? (
                  <div
                    key={item.id}
                    className="my-1 border-t border-(--_dk-line)"
                    role="separator"
                  />
                ) : (
                  <button
                    key={item.id}
                    type="button"
                    role="menuitem"
                    tabIndex={-1}
                    disabled={item.disabled}
                    className={`flex w-full items-center justify-between gap-4 px-3 py-1 text-left disabled:opacity-40 ${
                      item.danger
                        ? "text-(--_dk-ix-danger-fg) hover:bg-(--_dk-ix-danger-bg-hover)"
                        : "text-(--_dk-text-secondary) hover:bg-(--_dk-ix-bg-hover)"
                    }`}
                    onClick={() => {
                      if (item.disabled) return;
                      const restore = item.restoreFocus ?? true;
                      item.onClick?.();
                      if (restore) closeWithRestore();
                      else closeWithoutRestore();
                    }}
                  >
                    <span>{item.label}</span>
                    {item.shortcut && (
                      <span className="text-dk-2xs text-(--_dk-text-disabled)">
                        {item.shortcut}
                      </span>
                    )}
                  </button>
                ),
              )}
            </div>,
            target,
          )
        : null}
    </>
  );
}

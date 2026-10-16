import { useEffect, useRef, type RefObject } from "react";

import { viewOf } from "../../lib/domView";

interface FloatingLayerOptions {
  open: boolean;
  /** Wrapper around the trigger; clicks inside it never dismiss. */
  rootRef: RefObject<HTMLElement | null>;
  /** The portaled panel; clicks inside it never dismiss. */
  panelRef: RefObject<HTMLElement | null>;
  /** Dismiss (Escape, or mousedown outside root and panel). */
  onDismiss: () => void;
  /** Any scroll (capture phase) while open. */
  onScroll: () => void;
  /** Window resize while open. */
  onResize: () => void;
}

/**
 * Shared floating-panel mechanics for Dropdown and Popover: while open, listen
 * for scroll (capture) / resize to reposition, and dismiss on Escape or an
 * outside mousedown. Listeners attach to the trigger's own window/document so
 * panels in a popped-out window behave the same. Handlers are read through a
 * ref, so callers can pass fresh closures every render.
 */
export function useFloatingLayer({
  open,
  rootRef,
  panelRef,
  onDismiss,
  onScroll,
  onResize,
}: FloatingLayerOptions): void {
  const handlers = useRef({ onDismiss, onScroll, onResize });
  handlers.current = { onDismiss, onScroll, onResize };

  useEffect(() => {
    if (!open) return;
    const root = rootRef.current;
    const view = viewOf(root);
    const doc = root?.ownerDocument ?? document;
    const scroll = () => handlers.current.onScroll();
    const resize = () => handlers.current.onResize();
    const key = (e: KeyboardEvent) => {
      if (e.key === "Escape") handlers.current.onDismiss();
    };
    const down = (e: MouseEvent) => {
      const t = e.target as Node;
      if (rootRef.current?.contains(t)) return;
      if (panelRef.current?.contains(t)) return;
      handlers.current.onDismiss();
    };
    view.addEventListener("scroll", scroll, true);
    view.addEventListener("resize", resize);
    view.addEventListener("keydown", key);
    doc.addEventListener("mousedown", down);
    return () => {
      view.removeEventListener("scroll", scroll, true);
      view.removeEventListener("resize", resize);
      view.removeEventListener("keydown", key);
      doc.removeEventListener("mousedown", down);
    };
  }, [open, rootRef, panelRef]);
}

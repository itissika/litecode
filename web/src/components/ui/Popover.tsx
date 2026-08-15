import { type ReactNode, useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";

export interface PopoverApi {
  open: boolean;
  toggle: () => void;
  close: () => void;
}

interface PopoverProps {
  /** Where the floating panel anchors relative to the trigger. */
  placement?: "up-right" | "up-left" | "down-right" | "down-left";
  /** Fixed pixel width of the panel. */
  width?: number;
  /** Gap in px between trigger and panel. */
  gap?: number;
  /** Classes for the panel shell; merged after the default shell. */
  panelClassName?: string;
  /** Trigger renderer. Receives open state + toggle. */
  trigger: (api: { open: boolean; toggle: () => void }) => ReactNode;
  /** Panel content. Receives a close function. */
  children: ReactNode | ((api: { close: () => void }) => ReactNode);
}

const DEFAULT_GAP = 8;
const DEFAULT_WIDTH = 240;

/**
 * Portal-based floating panel primitive.
 *
 * Owns only the mechanics: open/close state, viewport-anchored positioning
 * (fixed + getBoundingClientRect, repositioned on scroll/resize), and
 * dismissal (toggle re-click, outside mousedown, blur, Escape). It renders no
 * content of its own — callers inject both the trigger and the panel body, so
 * the same primitive backs notification panels, context-usage popovers, etc.
 */
export function Popover({
  placement = "up-right",
  width = DEFAULT_WIDTH,
  gap = DEFAULT_GAP,
  panelClassName = "",
  trigger,
  children,
}: PopoverProps) {
  const [open, setOpen] = useState(false);
  const [pos, setPos] = useState<{
    top?: number;
    right?: number;
    bottom?: number;
    left?: number;
  } | null>(null);
  const rootRef = useRef<HTMLDivElement>(null);

  const update = () => {
    const el = rootRef.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    let next: { top?: number; right?: number; bottom?: number; left?: number };
    switch (placement) {
      case "up-left":
        next = { left: rect.left, bottom: vh - rect.top + gap };
        break;
      case "down-right":
        next = { right: vw - rect.right, top: rect.bottom + gap };
        break;
      case "down-left":
        next = { left: rect.left, top: rect.bottom + gap };
        break;
      case "up-right":
      default:
        next = { right: vw - rect.right, bottom: vh - rect.top + gap };
        break;
    }
    setPos(next);
  };

  useLayoutEffect(() => {
    if (!open) {
      setPos(null);
      return;
    }
    update();
  }, [open, gap]);

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
      const panel = document.getElementById(POPOVER_PANEL_ID);
      if (panel && panel.contains(t)) return;
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

  const toggle = () => setOpen((o) => !o);
  const close = () => setOpen(false);

  return (
    <div ref={rootRef} className="relative inline-flex">
      {trigger({
        open,
        toggle,
      })}
      {open &&
        pos &&
        createPortal(
          <div
            id={POPOVER_PANEL_ID}
            className={`fixed z-1000 border border-(--_dk-line-visible) bg-(--_dk-overlay) shadow-[0_6px_18px_rgba(0,0,0,0.18)] ${panelClassName}`}
            style={{ ...pos, width }}
          >
            {typeof children === "function" ? children({ close }) : children}
          </div>,
          document.body,
        )}
    </div>
  );
}

const POPOVER_PANEL_ID = "dk-popover-panel";

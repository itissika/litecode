import { useRef, useState, useCallback, useEffect, useLayoutEffect } from "react";

interface FloatingDialogProps {
  visible: boolean;
  title?: string;
  onClose: () => void;
  children?: React.ReactNode;
  defaultWidth?: number;
  defaultHeight?: number;
}

const MIN_W = 300;
const MIN_H = 200;

export function FloatingDialog({ visible, title = "Dialog", onClose, children, defaultWidth = 480, defaultHeight = 320 }: FloatingDialogProps) {
  const [pos, setPos] = useState({ x: 120, y: 60 });
  const [size, setSize] = useState({ w: defaultWidth, h: defaultHeight });
  const containerRef = useRef<HTMLDivElement>(null);

  const posRef = useRef(pos);
  const sizeRef = useRef(size);
  posRef.current = pos;
  sizeRef.current = size;

  // Size is stored as a fraction of the viewport (not absolute pixels) so the
  // panel keeps its relative footprint when the window is resized. Updated on
  // open and whenever the size changes (open / viewport resize / manual drag).
  const ratioRef = useRef({ rw: 0, rh: 0 });

  const dragState = useRef<{ startX: number; startY: number; origX: number; origY: number } | null>(null);
  const resizeState = useRef<{ startX: number; startY: number; origW: number; origH: number } | null>(null);

  const getBounds = useCallback(() => {
    const el = containerRef.current?.parentElement;
    if (!el) return { w: 0, h: 0 };
    return { w: el.clientWidth, h: el.clientHeight };
  }, []);

  // Keep the whole panel (and therefore its titlebar/head) inside the viewport.
  // The only goal is to never let the grabbable head leave the screen — no
  // "dead zone" math, no inverted ranges that can collapse the panel.
  const clampPosFor = (x: number, y: number, w: number, h: number) => {
    const b = getBounds();
    return {
      x: Math.max(0, Math.min(x, b.w - w)),
      y: Math.max(0, Math.min(y, b.h - h)),
    };
  };

  const clampPos = (x: number, y: number) =>
    clampPosFor(x, y, sizeRef.current.w, sizeRef.current.h);

  const clampSize = (w: number, h: number) => {
    const b = getBounds();
    const maxW = Math.min(b.w * 0.9, b.w - 20);
    const maxH = Math.min(b.h * 0.9, b.h - 20);
    return {
      w: Math.max(MIN_W, Math.min(w, maxW)),
      h: Math.max(MIN_H, Math.min(h, maxH)),
    };
  };

  // Reset to a consistent, viewport-relative default layout every time the
  // dialog is opened. Preferred pixel size is capped at 90% of the viewport so
  // it never overflows on small screens and stays consistent across displays.
  // useLayoutEffect (not useEffect) applies this before the browser paints, so
  // there is no one-frame flash of the previous position/size on open.
  useLayoutEffect(() => {
    if (!visible) return;
    const b = getBounds();
    const w = Math.min(defaultWidth, Math.round(b.w * 0.9));
    const h = Math.min(defaultHeight, Math.round(b.h * 0.9));
    setSize({ w, h });
    ratioRef.current = { rw: w / b.w, rh: h / b.h };
    setPos({ x: Math.round((b.w - w) / 2), y: Math.round((b.h - h) / 2) });
  }, [visible]);

  // On viewport resize, recompute the pixel size from the stored viewport ratio
  // so the panel keeps the same proportional footprint, then clamp to bounds
  // (min/max) and re-clamp the ratio from the resulting size.
  useEffect(() => {
    if (!visible) return;
    const onResize = () => {
      const b = getBounds();
      const pw = Math.round(ratioRef.current.rw * b.w);
      const ph = Math.round(ratioRef.current.rh * b.h);
      const ns = clampSize(pw, ph);
      ratioRef.current = { rw: ns.w / b.w, rh: ns.h / b.h };
      const np = clampPosFor(posRef.current.x, posRef.current.y, ns.w, ns.h);
      setSize(ns);
      setPos(np);
    };
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, [visible]);

  useEffect(() => {
    const onMove = (ev: MouseEvent) => {
      const d = dragState.current;
      if (!d) return;
      const rawX = d.origX + (ev.clientX - d.startX);
      const rawY = d.origY + (ev.clientY - d.startY);
      setPos(clampPos(rawX, rawY));
    };
    const onUp = () => {
      dragState.current = null;
      document.removeEventListener("mousemove", onMove);
      document.removeEventListener("mouseup", onUp);
    };

    const el = containerRef.current;
    if (!el) return;

    const onDragStart = (e: MouseEvent) => {
      e.preventDefault();
      dragState.current = {
        startX: e.clientX,
        startY: e.clientY,
        origX: posRef.current.x,
        origY: posRef.current.y,
      };
      document.addEventListener("mousemove", onMove);
      document.addEventListener("mouseup", onUp);
    };

    const titlebar = el.querySelector("[data-drag-handle]");
    titlebar?.addEventListener("mousedown", onDragStart as EventListener);
    return () => titlebar?.removeEventListener("mousedown", onDragStart as EventListener);
  }, [visible]);

  useEffect(() => {
    const onMove = (ev: MouseEvent) => {
      const d = resizeState.current;
      if (!d) return;
      const newSize = clampSize(d.origW + (ev.clientX - d.startX), d.origH + (ev.clientY - d.startY));
      setSize(newSize);
      const b = getBounds();
      ratioRef.current = { rw: newSize.w / b.w, rh: newSize.h / b.h };
      setPos((p) => clampPosFor(p.x, p.y, newSize.w, newSize.h));
    };
    const onUp = () => {
      resizeState.current = null;
      document.removeEventListener("mousemove", onMove);
      document.removeEventListener("mouseup", onUp);
    };

    const el = containerRef.current;
    if (!el) return;

    const onResizeStart = (e: MouseEvent) => {
      e.preventDefault();
      e.stopPropagation();
      resizeState.current = {
        startX: e.clientX,
        startY: e.clientY,
        origW: sizeRef.current.w,
        origH: sizeRef.current.h,
      };
      document.addEventListener("mousemove", onMove);
      document.addEventListener("mouseup", onUp);
    };

    const handle = el.querySelector("[data-resize-handle]");
    handle?.addEventListener("mousedown", onResizeStart as EventListener);
    return () => handle?.removeEventListener("mousedown", onResizeStart as EventListener);
  }, [visible]);

  if (!visible) return null;

  return (
    <div
      ref={containerRef}
      className="lc-dialog absolute z-[9999] flex flex-col select-none"
      style={{
        left: pos.x,
        top: pos.y,
        width: size.w,
        height: size.h,
      }}
    >
      <div data-drag-handle className="lc-dialog-titlebar">
        <span className="lc-dialog-title">{title}</span>
        <button
          className="lc-dialog-close"
          onMouseDown={(e) => e.stopPropagation()}
          onClick={onClose}
          aria-label="Close dialog"
        >
          <svg width="10" height="10" viewBox="0 0 10 10">
            <path d="M1 1l8 8M9 1L1 9" stroke="currentColor" strokeWidth="1.5" />
          </svg>
        </button>
      </div>

      <div className="lc-dialog-body">{children}</div>

      <div data-resize-handle className="lc-dialog-resize" />
    </div>
  );
}

import { useRef, useState, type MouseEvent, type RefObject } from "react";

const MIN = 0.2;
const MAX = 0.8;

function clamp(v: number): number {
  return Math.min(MAX, Math.max(MIN, v));
}

function readSplit(key: string, fallback: number): number {
  const raw = Number(localStorage.getItem(key));
  if (!Number.isFinite(raw)) return fallback;
  return clamp(raw);
}

/**
 * Persisted top/bottom split ratio (0.2–0.8) with a mouse-drag divider.
 * The ratio is stored under `storageKey` when the drag ends.
 */
export function useVerticalSplit(
  storageKey: string,
  defaultRatio: number,
): {
  split: number;
  paneRef: RefObject<HTMLDivElement | null>;
  onSplitterDown: (e: MouseEvent) => void;
} {
  const [split, setSplit] = useState(() => readSplit(storageKey, defaultRatio));
  const splitRef = useRef(split);
  splitRef.current = split;
  const paneRef = useRef<HTMLDivElement>(null);

  const onSplitterDown = (e: MouseEvent) => {
    e.preventDefault();
    const pane = paneRef.current;
    if (!pane) return;
    const startY = e.clientY;
    const start = splitRef.current;
    const height = pane.getBoundingClientRect().height;
    const onMove = (ev: globalThis.MouseEvent) => {
      if (height <= 0) return;
      const next = clamp(start + (ev.clientY - startY) / height);
      splitRef.current = next;
      setSplit(next);
    };
    const onUp = () => {
      localStorage.setItem(storageKey, String(splitRef.current));
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  };

  return { split, paneRef, onSplitterDown };
}

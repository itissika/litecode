import {
  type DependencyList,
  type RefObject,
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";

import { hostResizeObserver, viewOf } from "../../lib/domView";
import { dockPane } from "./dockPane";

/** Fixed-position fields shared by every portaled panel. */
export interface PanelPosition {
  top?: number;
  bottom?: number;
  left?: number;
  right?: number;
  width?: number;
  minWidth?: number;
  maxWidth?: number;
  maxHeight?: number;
}

function samePosition(a: PanelPosition | null, b: PanelPosition): boolean {
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

interface UseAnchoredPanelOptions<T extends PanelPosition> {
  open: boolean;
  /** Wrapper around the trigger; its rect anchors the panel. */
  rootRef: RefObject<HTMLElement | null>;
  /** Return the next fixed-position style, or null when the trigger is gone. */
  compute: (el: HTMLElement, view: Window) => T | null;
  /** Recompute when these trigger inputs change, not on every render. */
  deps: DependencyList;
  /** Called when compute reports that the panel can no longer be placed. */
  onUnavailable?: () => void;
}

interface UseAnchoredPanelResult<T extends PanelPosition> {
  pos: T | null;
  /** Stable reposition function for scroll/resize/flip observers. */
  update: () => void;
}

/**
 * Owns the position state for an anchored, portaled panel.
 *
 * The caller supplies geometry; this hook owns the parts that were duplicated
 * by Dropdown and Popover: reset on close, equality-guarded state updates, and
 * re-clamping when the dock pane resizes. It deliberately does not own visual
 * presets or open state.
 */
export function useAnchoredPanel<T extends PanelPosition>({
  open,
  rootRef,
  compute,
  deps,
  onUnavailable,
}: UseAnchoredPanelOptions<T>): UseAnchoredPanelResult<T> {
  const [pos, setPos] = useState<T | null>(null);
  const computeRef = useRef(compute);
  const unavailableRef = useRef(onUnavailable);
  computeRef.current = compute;
  unavailableRef.current = onUnavailable;

  const update = useCallback(() => {
    const el = rootRef.current;
    if (!el) {
      setPos(null);
      return;
    }
    const next = computeRef.current(el, viewOf(el));
    if (!next) {
      setPos(null);
      unavailableRef.current?.();
      return;
    }
    setPos((prev) => (samePosition(prev, next) ? prev : next));
  }, [rootRef]);

  useLayoutEffect(() => {
    if (!open) {
      setPos(null);
      return;
    }
    update();
    // The consumer's deps are trigger inputs (direction, width, max height…).
    // update is stable; spreading keeps the caller's dependency list honest.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, update, ...deps]);

  // Sash drags do not fire window resize. Both panels need this re-clamp;
  // use the trigger's window constructor so popouts keep working.
  useEffect(() => {
    if (!open) return;
    const el = rootRef.current;
    if (!el) return;
    const pane = dockPane(el);
    const Observer = hostResizeObserver(el);
    if (!pane || !Observer) return;
    const observer = new Observer(() => update());
    observer.observe(pane);
    return () => observer.disconnect();
  }, [open, rootRef, update]);

  return { pos, update };
}

import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type DragEvent,
  type DragEventHandler,
} from "react";

export type DropPosition = "before" | "inside" | "after";

export interface DropSpec<T> {
  /** Wire MIME this target accepts. */
  mime: string;
  /** Parses the payload on drop; return null for a malformed/foreign drag. */
  parse: (raw: string) => T | null;
  /** Where this row would receive the item. Defaults to "inside". */
  resolve?: (event: DragEvent<HTMLElement>, rect: DOMRect) => DropPosition;
  canDrop?: (payload: T, position: DropPosition) => boolean;
  onDrop: (
    payload: T,
    position: DropPosition,
    event: DragEvent<HTMLElement>,
  ) => void;
  effect?: "move" | "copy";
}

export interface DropTargetState {
  isOver: boolean;
  position: DropPosition | null;
  handlers: {
    onDragEnter: DragEventHandler<HTMLElement>;
    onDragOver: DragEventHandler<HTMLElement>;
    onDragLeave: DragEventHandler<HTMLElement>;
    onDrop: DragEventHandler<HTMLElement>;
  };
}

/** One window listener for every target, instead of one per row. */
const resets = new Set<() => void>();
let attached = false;

function resetAll() {
  for (const reset of resets) reset();
}

function subscribeReset(reset: () => void): () => void {
  resets.add(reset);
  if (!attached) {
    window.addEventListener("dragend", resetAll);
    attached = true;
  }
  return () => {
    resets.delete(reset);
    if (resets.size === 0 && attached) {
      window.removeEventListener("dragend", resetAll);
      attached = false;
    }
  };
}

/**
 * Shared drop mechanics: accept by `dataTransfer.types`, prevent the browser
 * from rejecting the drop, keep hover/leave stable across child elements,
 * resolve before/inside/after from the row rect, then hand a typed payload to
 * the domain.
 */
export function useDropTarget<T>(
  specs: DropSpec<T> | DropSpec<T>[],
): DropTargetState {
  const list = Array.isArray(specs) ? specs : [specs];
  const specsRef = useRef(list);
  specsRef.current = list;
  const [position, setPosition] = useState<DropPosition | null>(null);

  const findSpec = useCallback(
    (event: DragEvent<HTMLElement>): DropSpec<T> | undefined => {
      const types = Array.from(event.dataTransfer.types);
      return specsRef.current.find((spec) => types.includes(spec.mime));
    },
    [],
  );

  const resolve = useCallback(
    (spec: DropSpec<T>, event: DragEvent<HTMLElement>): DropPosition =>
      spec.resolve?.(event, event.currentTarget.getBoundingClientRect()) ??
      "inside",
    [],
  );

  const clear = useCallback(() => setPosition(null), []);

  useEffect(() => subscribeReset(clear), [clear]);

  const onDragEnter = useCallback(
    (event: DragEvent<HTMLElement>) => {
      const spec = findSpec(event);
      if (!spec) return;
      event.preventDefault();
      event.stopPropagation();
      setPosition(resolve(spec, event));
    },
    [findSpec, resolve],
  );

  const onDragOver = useCallback(
    (event: DragEvent<HTMLElement>) => {
      const spec = findSpec(event);
      if (!spec) return;
      event.preventDefault();
      event.stopPropagation();
      event.dataTransfer.dropEffect = spec.effect ?? "move";
      const next = resolve(spec, event);
      setPosition((current) => (current === next ? current : next));
    },
    [findSpec, resolve],
  );

  const onDragLeave = useCallback((event: DragEvent<HTMLElement>) => {
    const next = event.relatedTarget;
    if (next instanceof Node && event.currentTarget.contains(next)) return;
    setPosition(null);
  }, []);

  const onDrop = useCallback(
    (event: DragEvent<HTMLElement>) => {
      const spec = findSpec(event);
      if (!spec) return;
      event.preventDefault();
      event.stopPropagation();
      const next = resolve(spec, event);
      let payload: T | null = null;
      try {
        payload = spec.parse(event.dataTransfer.getData(spec.mime));
      } catch {
        payload = null;
      }
      setPosition(null);
      if (payload == null) return;
      if (spec.canDrop && !spec.canDrop(payload, next)) return;
      spec.onDrop(payload, next, event);
    },
    [findSpec, resolve],
  );

  return {
    isOver: position !== null,
    position,
    handlers: { onDragEnter, onDragOver, onDragLeave, onDrop },
  };
}

import { useCallback, type DragEvent, type DragEventHandler } from "react";

export interface DragSourceOptions<T> {
  /** Wire MIME; also the key drop targets match on during dragover. */
  mime: string;
  /** Called once at dragstart; encode the current row/selection here. */
  payload: () => T;
  canDrag?: boolean;
  effect?: "move" | "copy";
  /** Plain-text fallback for native targets such as the mention picker. */
  text?: (payload: T) => string;
  onStart?: (payload: T) => void;
  onEnd?: () => void;
}

export interface DragSourceState {
  draggable: boolean;
  onDragStart: DragEventHandler<HTMLElement>;
  onDragEnd: DragEventHandler<HTMLElement>;
}

/** One drag source implementation for every row: MIME, payload, effect, end. */
export function useDragSource<T>({
  mime,
  payload,
  canDrag = true,
  effect = "move",
  text,
  onStart,
  onEnd,
}: DragSourceOptions<T>): DragSourceState {
  const onDragStart = useCallback(
    (event: DragEvent<HTMLElement>) => {
      if (!canDrag) {
        event.preventDefault();
        return;
      }
      const data = payload();
      event.dataTransfer.effectAllowed = effect;
      event.dataTransfer.setData(mime, JSON.stringify(data));
      if (text) event.dataTransfer.setData("text/plain", text(data));
      onStart?.(data);
    },
    [canDrag, effect, mime, onStart, payload, text],
  );

  const onDragEnd = useCallback(() => {
    onEnd?.();
  }, [onEnd]);

  return { draggable: canDrag, onDragStart, onDragEnd };
}

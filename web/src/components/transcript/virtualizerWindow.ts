import { observeHostSize, subscribeHostDocument } from "../../lib/hostDocument";

type Rebindable = {
  scrollElement: HTMLElement | null;
  targetWindow: (Window & typeof globalThis) | null;
  _willUpdate: () => void;
};

/**
 * Dockview moves the scroller into a popout without changing the node.
 * Virtualizer only rebinds when that identity changes, so its ResizeObserver
 * stays on the opener and never reports the minichat's new height. The row
 * stays short, the editor paints over the next row, and that row takes the click.
 */
export function rebindVirtualizerWindow(virtualizer: Rebindable): void {
  const current = virtualizer.scrollElement;
  if (!current || !("ownerDocument" in current)) return;
  const next = current.ownerDocument.defaultView;
  if (!next || next.closed || virtualizer.targetWindow === next) return;
  virtualizer.scrollElement = null;
  virtualizer._willUpdate();
}

/** Calls `onMove` now and again whenever `scroller` changes documents. */
export function followScrollerWindow(
  scroller: HTMLElement,
  onMove: () => void,
): () => void {
  return subscribeHostDocument(scroller, onMove);
}

/** Keep one virtual row equal to `item`'s border box, using that item's window. */
export function observeRowHeight(
  item: HTMLElement,
  onHeight: (height: number) => void,
): () => void {
  return observeHostSize(item, () => onHeight(item.offsetHeight));
}

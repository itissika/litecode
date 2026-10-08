import { hostResizeObserver } from "../../lib/domView";

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
  let observer: MutationObserver | null = null;
  let stopped = false;

  const arm = (doc: Document) => {
    observer?.disconnect();
    const Ctor = doc.defaultView?.MutationObserver ?? MutationObserver;
    observer = new Ctor(() => {
      if (stopped) return;
      if (scroller.isConnected && scroller.ownerDocument === doc) return;
      if (!scroller.isConnected) return;
      const next = scroller.ownerDocument;
      const view = next.defaultView;
      const run = () => {
        if (stopped || !scroller.isConnected) return;
        onMove();
        arm(next);
      };
      if (view) view.requestAnimationFrame(run);
      else run();
    });
    const root = doc.documentElement;
    if (root) observer.observe(root, { childList: true, subtree: true });
  };

  onMove();
  arm(scroller.ownerDocument);
  return () => {
    stopped = true;
    observer?.disconnect();
  };
}

/** Keep one virtual row equal to `item`'s border box, using that item's window. */
export function observeRowHeight(
  item: HTMLElement,
  onHeight: (height: number) => void,
): () => void {
  const report = () => onHeight(item.offsetHeight);
  report();
  const Observer = hostResizeObserver(item);
  if (!Observer) return () => {};
  const observer = new Observer(report);
  observer.observe(item);
  return () => observer.disconnect();
}

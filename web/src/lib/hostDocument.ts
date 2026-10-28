import { hostResizeObserver, viewOf } from "./domView";

/**
 * Document-level lifecycle for nodes that belong to a panel.
 *
 * Dockview moves a panel's DOM into a popout window without recreating it:
 * same node, same JS realm, a different Document. Anything that measured,
 * observed, or listened through the opener stops working at that moment.
 * Components used to detect this with their own MutationObserver (the
 * ProseMirror view and the transcript virtualizer each had one) and to grab
 * a ResizeObserver once. Both belong here instead.
 */

/**
 * Call `listener` now with the window that owns `node`, then again whenever
 * `node` moves to another document. Watch the owning document's root, since a
 * reparent into a popout removes the node from this one first.
 * Returns a stop function; safe for a null node.
 */
export function subscribeHostDocument(
  node: Node | null | undefined,
  listener: (win: Window) => void,
): () => void {
  if (!node) return () => {};
  let stopped = false;
  let observer: MutationObserver | null = null;
  let armedDoc: Document | null = null;

  const arm = (doc: Document) => {
    observer?.disconnect();
    armedDoc = doc;
    const Ctor = doc.defaultView?.MutationObserver ?? MutationObserver;
    observer = new Ctor(() => {
      if (stopped) return;
      const next = node.ownerDocument;
      if (!next || next === armedDoc || !node.isConnected) return;
      const win = next.defaultView;
      if (!win || win.closed) return;
      // Report in the observer turn, not after a frame: callers that only
      // rebind listeners (ProseMirror) want the move now, and callers that
      // measure re-observe the node, which fires its own first report.
      listener(win);
      arm(next);
    });
    const root = doc.documentElement;
    if (root) observer.observe(root, { childList: true, subtree: true });
  };

  listener(viewOf(node));
  arm(node.ownerDocument ?? document);

  return () => {
    stopped = true;
    observer?.disconnect();
    observer = null;
  };
}

/**
 * Fire now, on every host-document change, and on every size change of \`node\`.
 * The ResizeObserver comes from the window that currently owns the node, so a
 * popout move re-creates it instead of leaving a dead observer behind.
 */
export function observeHostSize(
  node: Element | null | undefined,
  listener: () => void,
): () => void {
  if (!node) return () => {};
  let stopped = false;
  let observer: ResizeObserver | null = null;

  const rebind = () => {
    observer?.disconnect();
    observer = null;
    const Ctor = hostResizeObserver(node);
    if (Ctor) {
      observer = new Ctor(() => {
        if (!stopped) listener();
      });
      observer.observe(node);
    }
    if (!stopped) listener();
  };

  const stopDoc = subscribeHostDocument(node, rebind);
  return () => {
    stopped = true;
    stopDoc();
    observer?.disconnect();
    observer = null;
  };
}

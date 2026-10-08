import type { EditorView } from "@tiptap/pm/view";

type SelectionObserver = {
  onSelectionChange: EventListener;
  stop(): void;
  start(): void;
};

type Bucket = {
  observer: MutationObserver;
  views: Set<EditorView>;
};

const buckets = new Map<Document, Bucket>();
const boundDoc = new WeakMap<EditorView, Document>();
const afterMove = new WeakMap<EditorView, () => void>();

function selectionObserver(view: EditorView): SelectionObserver {
  return (view as EditorView & { domObserver: SelectionObserver }).domObserver;
}

/**
 * ProseMirror remembers the document it was created in. Dockview reparents a
 * panel into a popout window without recreating the editor, so selection
 * stays on the opener until the view is told to follow that document.
 * `updateRoot` only clears the cache; the selection listener stays behind.
 */
export function rebindEditorView(view: EditorView): void {
  if (view.isDestroyed) return;
  const dom = view.dom;
  if (!dom.isConnected) return;
  const next = dom.ownerDocument;
  const previous = boundDoc.get(view);
  if (previous === next) return;

  const observer = selectionObserver(view);
  if (previous) previous.removeEventListener("selectionchange", observer.onSelectionChange);
  // The view is constructed in this realm. Its first listener sits on the
  // opener document even after the node has moved.
  if (document !== next) {
    document.removeEventListener("selectionchange", observer.onSelectionChange);
  }
  view.updateRoot();
  observer.stop();
  observer.start();
  boundDoc.set(view, next);
  if (previous) afterMove.get(view)?.();
}

/** Follow later reparents. The first call also catches a view already in a popout. */
export function trackEditorView(view: EditorView, onMove?: () => void): () => void {
  if (onMove) afterMove.set(view, onMove);
  rebindEditorView(view);
  watch(view);
  return () => {
    afterMove.delete(view);
    unwatch(view);
  };
}

function watch(view: EditorView): void {
  const doc = view.dom.ownerDocument;
  let bucket = buckets.get(doc);
  if (!bucket) {
    const views = new Set<EditorView>();
    const observer = new MutationObserver(() => {
      for (const tracked of views) {
        if (tracked.isDestroyed) {
          views.delete(tracked);
          continue;
        }
        if (!tracked.dom.isConnected || tracked.dom.ownerDocument === doc) continue;
        views.delete(tracked);
        try {
          rebindEditorView(tracked);
        } finally {
          if (!tracked.isDestroyed && tracked.dom.isConnected) watch(tracked);
        }
      }
      if (views.size === 0) {
        observer.disconnect();
        buckets.delete(doc);
      }
    });
    observer.observe(doc.documentElement, { childList: true, subtree: true });
    bucket = { observer, views };
    buckets.set(doc, bucket);
  }
  bucket.views.add(view);
}

function unwatch(view: EditorView): void {
  for (const [doc, bucket] of buckets) {
    if (!bucket.views.delete(view)) continue;
    if (bucket.views.size === 0) {
      bucket.observer.disconnect();
      buckets.delete(doc);
    }
  }
}

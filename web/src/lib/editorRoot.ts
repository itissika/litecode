import type { EditorView } from "@tiptap/pm/view";

import { subscribeHostDocument } from "./hostDocument";

type SelectionObserver = {
  onSelectionChange: EventListener;
  stop(): void;
  start(): void;
};

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
function rebindEditorView(view: EditorView): void {
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
  // The subscription reports the current document first, which binds a view
  // that is already in a popout.
  const stop = subscribeHostDocument(view.dom, () => {
    if (!view.isDestroyed) rebindEditorView(view);
  });
  return () => {
    afterMove.delete(view);
    stop();
  };
}

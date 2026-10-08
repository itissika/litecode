/**
 * The window that contains `node`.
 * A popped-out panel lives in another document. Measurement, animation
 * frames, and document listeners have to use that window. The opener's
 * global `window` is in the background and drops those signals.
 */
export function viewOf(node: Node | null | undefined): Window {
  const view = node?.ownerDocument?.defaultView;
  if (view && !view.closed) return view;
  return window;
}

/** DOM constructors live on each window. The opener's `Window` type does not list them. */
type DomWindow = Window & {
  ResizeObserver?: typeof ResizeObserver;
  Element: typeof Element;
  HTMLElement: typeof HTMLElement;
  Node: typeof Node;
};

function domWindow(node: Node | null | undefined): DomWindow {
  return viewOf(node) as DomWindow;
}

/** ResizeObserver from the window that contains `node`. */
export function hostResizeObserver(
  node: Node | null | undefined,
): typeof ResizeObserver | null {
  return domWindow(node).ResizeObserver ?? null;
}

/**
 * A node ProseMirror builds with the opener `document` and then appends into
 * a popout keeps the opener's prototype. `instanceof` the popout's Element
 * is false even though `ownerDocument` is the popout. Hit-testing has to use
 * the document, or a click on that text looks like an outside click.
 */
function hostNode(target: EventTarget | null, scope: Node | null | undefined): Node | null {
  if (!target || typeof target !== "object") return null;
  const node = target as Node;
  if (node.nodeType !== 1 && node.nodeType !== 3) return null;
  const doc = scope?.ownerDocument ?? document;
  return node.ownerDocument === doc ? node : null;
}

/** Element in `scope`'s document. Moved-in nodes are included. */
export function isHostElement(
  target: EventTarget | null,
  scope: Node | null | undefined,
): target is Element {
  const node = hostNode(target, scope);
  return node?.nodeType === 1;
}

/**
 * Element under a hit in `scope`'s document.
 * A caret click's target is often the text node itself.
 */
export function hostElementFromTarget(
  target: EventTarget | null,
  scope: Node | null | undefined,
): Element | null {
  const node = hostNode(target, scope);
  if (!node) return null;
  return node.nodeType === 1 ? (node as Element) : node.parentElement;
}

/** HTMLElement from `scope`'s document, or null. */
export function hostHtmlElement(
  node: Element | null,
  scope: Node | null | undefined,
): HTMLElement | null {
  if (!node || !isHostElement(node, scope)) return null;
  return node as HTMLElement;
}

/** Resize cursor on the document that owns `node`, not the opener. */
export function paintDragCursor(node: Node | null | undefined, active: boolean): void {
  const body = (node?.ownerDocument ?? document).body;
  if (!body) return;
  body.style.userSelect = active ? "none" : "";
  body.style.cursor = active ? "ns-resize" : "";
}

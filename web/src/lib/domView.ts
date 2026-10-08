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

/** `instanceof Element` against the window that contains `scope`, not the opener. */
export function isHostElement(
  target: EventTarget | null,
  scope: Node | null | undefined,
): target is Element {
  return !!target && target instanceof domWindow(scope).Element;
}

/** HTMLElement from `scope`'s window, or null. */
export function hostHtmlElement(
  node: Element | null,
  scope: Node | null | undefined,
): HTMLElement | null {
  if (!node) return null;
  return node instanceof domWindow(scope).HTMLElement ? node : null;
}

/** Resize cursor on the document that owns `node`, not the opener. */
export function paintDragCursor(node: Node | null | undefined, active: boolean): void {
  const body = (node?.ownerDocument ?? document).body;
  if (!body) return;
  body.style.userSelect = active ? "none" : "";
  body.style.cursor = active ? "ns-resize" : "";
}

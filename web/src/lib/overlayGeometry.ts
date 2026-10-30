export type OverlayRect = {
  left: number;
  top: number;
  right: number;
  bottom: number;
};

export type NativeViewKind = "browser" | "app";

export function nativeViewKey(kind: NativeViewKind, id: string): string {
  return `${kind}:${id}`;
}

export function rectsOverlap(a: OverlayRect, b: OverlayRect): boolean {
  return a.left < b.right && a.right > b.left && a.top < b.bottom && a.bottom > b.top;
}

type VisibilityAwareElement = HTMLElement & {
  checkVisibility?: (options?: {
    checkOpacity?: boolean;
    checkVisibilityCSS?: boolean;
  }) => boolean;
};

/**
 * A native view host can be inside a hidden dockview panel. Check visibility
 * on the element itself so the manager does not hide a view that is not
 * actually on screen.
 */
export function isElementVisible(element: HTMLElement): boolean {
  if (!element.isConnected) return false;

  const aware = element as VisibilityAwareElement;
  if (typeof aware.checkVisibility === "function") {
    return aware.checkVisibility({ checkOpacity: true, checkVisibilityCSS: true });
  }

  if (element.getClientRects().length === 0) return false;
  const view = element.ownerDocument.defaultView;
  if (!view) return false;
  const style = view.getComputedStyle(element);
  return (
    style.display !== "none" &&
    style.visibility !== "hidden" &&
    Number.parseFloat(style.opacity || "1") > 0
  );
}

/** Viewport-space rect. Returns null for an invisible or zero-sized element. */
export function elementRect(element: HTMLElement): OverlayRect | null {
  if (!isElementVisible(element)) return null;
  const rect = element.getBoundingClientRect();
  if (rect.width < 1 || rect.height < 1) return null;
  return { left: rect.left, top: rect.top, right: rect.right, bottom: rect.bottom };
}

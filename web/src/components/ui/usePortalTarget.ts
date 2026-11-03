import { useLayoutEffect, useRef, useState, type RefObject } from "react";

/**
 * Resolve the body of the document that owns this React tree.
 * A panel in a popout renders into a different document; assuming the opener
 * window would put menus/dialogs behind the wrong window.
 */
export function usePortalTarget<T extends HTMLElement>(): {
  anchorRef: RefObject<T | null>;
  target: HTMLElement | null;
} {
  const anchorRef = useRef<T>(null);
  const [target, setTarget] = useState<HTMLElement | null>(null);

  useLayoutEffect(() => {
    const anchor = anchorRef.current;
    if (!anchor) return;
    setTarget(anchor.ownerDocument.body);
  }, []);

  return { anchorRef, target };
}

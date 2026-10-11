import { useCallback, useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";

import { viewOf } from "../../lib/domView";
import {
  IMAGE_CHIP_PREVIEW_DELAY_MS,
  IMAGE_CHIP_PREVIEW_HEIGHT,
  IMAGE_CHIP_PREVIEW_WIDTH,
  loadImageChipPreview,
} from "./imageChipHover";

const GAP = 8;
const MARGIN = 8;

function placePreview(anchor: HTMLElement): { top: number; left: number } {
  const view = viewOf(anchor);
  const rect = anchor.getBoundingClientRect();
  let top = rect.top - IMAGE_CHIP_PREVIEW_HEIGHT - GAP;
  if (top < MARGIN) top = rect.bottom + GAP;
  if (top + IMAGE_CHIP_PREVIEW_HEIGHT > view.innerHeight - MARGIN) {
    top = Math.max(MARGIN, view.innerHeight - IMAGE_CHIP_PREVIEW_HEIGHT - MARGIN);
  }
  const maxLeft = Math.max(MARGIN, view.innerWidth - IMAGE_CHIP_PREVIEW_WIDTH - MARGIN);
  const left = Math.min(Math.max(rect.left, MARGIN), maxLeft);
  return { top, left };
}

/**
 * Fixed-size hover preview for one image chip. The capsule is `children`;
 * this layer does not change the citation. The panel is portaled into the
 * chip's own document so a popped-out editor does not paint it on the opener.
 */
export function ImageChipPreview({
  path,
  children,
}: {
  path: string;
  children: ReactNode;
}) {
  const anchorRef = useRef<HTMLSpanElement>(null);
  const openTimer = useRef<number | null>(null);
  const pathRef = useRef(path);
  const [open, setOpen] = useState(false);
  const [url, setUrl] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);
  const [box, setBox] = useState<{ top: number; left: number } | null>(null);

  const clearOpenTimer = useCallback(() => {
    const view = viewOf(anchorRef.current);
    if (openTimer.current === null) return;
    view.clearTimeout(openTimer.current);
    openTimer.current = null;
  }, []);

  const close = useCallback(() => {
    clearOpenTimer();
    setOpen(false);
    setBox(null);
  }, [clearOpenTimer]);

  useEffect(() => () => clearOpenTimer(), [clearOpenTimer]);

  useEffect(() => {
    if (pathRef.current === path) return;
    pathRef.current = path;
    clearOpenTimer();
    setUrl(null);
    setFailed(false);
    setOpen(false);
    setBox(null);
  }, [path, clearOpenTimer]);

  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    setFailed(false);
    loadImageChipPreview(path).then(
      (next) => {
        if (cancelled) return;
        setUrl(next);
        setFailed(false);
      },
      () => {
        if (cancelled) return;
        setUrl(null);
        setFailed(true);
      },
    );
    return () => {
      cancelled = true;
    };
  }, [open, path]);

  useLayoutEffect(() => {
    if (!open) return;
    const anchor = anchorRef.current;
    if (!anchor) return;
    const next = placePreview(anchor);
    setBox((prev) =>
      prev && prev.top === next.top && prev.left === next.left ? prev : next,
    );
    const view = viewOf(anchor);
    const onHide = () => close();
    view.addEventListener("scroll", onHide, true);
    view.addEventListener("resize", onHide);
    return () => {
      view.removeEventListener("scroll", onHide, true);
      view.removeEventListener("resize", onHide);
    };
  }, [open, close]);

  const onEnter = () => {
    if (open || openTimer.current !== null) return;
    const view = viewOf(anchorRef.current);
    openTimer.current = view.setTimeout(() => {
      openTimer.current = null;
      setOpen(true);
    }, IMAGE_CHIP_PREVIEW_DELAY_MS);
  };

  const doc = anchorRef.current?.ownerDocument ?? document;

  return (
    <span
      ref={anchorRef}
      className="image-chip-anchor"
      onMouseEnter={onEnter}
      onMouseLeave={close}
    >
      {children}
      {open && box
        ? createPortal(
            <div
              className="image-chip-preview"
              style={{
                top: box.top,
                left: box.left,
                width: IMAGE_CHIP_PREVIEW_WIDTH,
                height: IMAGE_CHIP_PREVIEW_HEIGHT,
              }}
              aria-hidden="true"
            >
              {url && !failed ? (
                <img src={url} alt="" onError={() => setFailed(true)} />
              ) : failed ? (
                <span className="image-chip-preview-note">Can't preview</span>
              ) : null}
            </div>,
            doc.body,
          )
        : null}
    </span>
  );
}

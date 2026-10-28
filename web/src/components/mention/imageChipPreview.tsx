import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type ReactNode,
  type SyntheticEvent,
} from "react";
import { createPortal } from "react-dom";

import { viewOf } from "../../lib/domView";
import { fileKindFromPath } from "../../lib/fileKind";
import { loadImageChipPreview } from "./imageChipHover";

const GAP = 8;
const MARGIN = 8;

function placePreview(
  anchor: HTMLElement,
  width: number,
  height: number,
): { top: number; left: number } {
  const view = viewOf(anchor);
  const rect = anchor.getBoundingClientRect();
  let top = rect.top - height - GAP;
  if (top < MARGIN) top = rect.bottom + GAP;
  if (top + height > view.innerHeight - MARGIN) {
    top = Math.max(MARGIN, view.innerHeight - height - MARGIN);
  }
  const maxLeft = Math.max(MARGIN, view.innerWidth - width - MARGIN);
  const left = Math.min(Math.max(rect.left, MARGIN), maxLeft);
  return { top, left };
}

/**
 * Hover layer for a citation chip. It opens as soon as the pointer enters.
 * A whole-file image or video keeps its own pixel size. CSS max-width and
 * max-height scale both edges together when either one is past the preview
 * cap. Gif, webp, and avif animate in the picture tag. Video
 * plays muted and looped. The full reference, when it is longer than the capsule, is the
 * caption in that same panel. The panel is portaled into the chip's own
 * document so a popped-out editor does not paint it on the opener. A scroll
 * elsewhere in that document (another session panel streaming to its end)
 * must leave it alone; only a scroll that can move the chip dismisses it.
 */
export function ImageChipPreview({
  path,
  text,
  className,
  children,
}: {
  path?: string | null;
  text?: string | null;
  className?: string;
  children: ReactNode;
}) {
  const anchorRef = useRef<HTMLSpanElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const shownRef = useRef({ path, text });
  const [open, setOpen] = useState(false);
  const [url, setUrl] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);
  const [ready, setReady] = useState(false);
  const [box, setBox] = useState<{ top: number; left: number } | null>(null);
  const caption = text?.trim() ? text : null;
  const active = Boolean(path || caption);

  const close = useCallback(() => {
    setOpen(false);
    setBox(null);
  }, []);

  useEffect(() => {
    if (shownRef.current.path === path && shownRef.current.text === text) return;
    shownRef.current = { path, text };
    setUrl(null);
    setFailed(false);
    setReady(false);
    setOpen(false);
    setBox(null);
  }, [path, text]);

  useEffect(() => {
    if (!open || !path) return;
    let cancelled = false;
    setFailed(false);
    setReady(false);
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
    const panel = panelRef.current;
    if (!anchor || !panel) return;
    const next = placePreview(anchor, panel.offsetWidth, panel.offsetHeight);
    setBox((prev) =>
      prev && prev.top === next.top && prev.left === next.left ? prev : next,
    );
    const view = viewOf(anchor);
    // Capture-phase scroll reaches the window from every panel's scroller:
    // scroll does not bubble, but it does traverse the capture path. A
    // non-bubbling scroll in another session panel is constant while it
    // streams, so it may hide this preview only when the target is the
    // document/window or a container the chip lives in -- i.e. when the
    // anchor itself could have moved.
    const onScroll = (event: Event) => {
      const target = event.target as Node | null;
      if (
        target &&
        typeof target.contains === "function" &&
        !target.contains(anchor)
      ) {
        return;
      }
      close();
    };
    const onHide = () => close();
    view.addEventListener("scroll", onScroll, true);
    view.addEventListener("resize", onHide);
    return () => {
      view.removeEventListener("scroll", onScroll, true);
      view.removeEventListener("resize", onHide);
    };
  }, [open, close, path, caption, failed, ready]);

  const onImageLoad = () => setReady(true);

  const onVideoReady = (event: SyntheticEvent<HTMLVideoElement>) => {
    setReady(true);
    void event.currentTarget.play().catch(() => {});
  };

  const onEnter = () => {
    if (!active || open) return;
    setOpen(true);
  };

  const doc = anchorRef.current?.ownerDocument ?? document;
  const shown = !path || failed || ready;
  const video = path != null && fileKindFromPath(path) === "video";
  const picture = path ? (
    url && !failed ? (
      video ? (
        <video
          src={url}
          muted
          autoPlay
          loop
          playsInline
          onLoadedMetadata={onVideoReady}
          onError={() => setFailed(true)}
        />
      ) : (
        <img src={url} alt="" onLoad={onImageLoad} onError={() => setFailed(true)} />
      )
    ) : failed ? (
      <span className="image-chip-preview-note">Can't preview</span>
    ) : null
  ) : null;

  return (
    <span
      ref={anchorRef}
      className={["image-chip-anchor", className].filter(Boolean).join(" ")}
      onMouseEnter={active ? onEnter : undefined}
      onMouseLeave={active ? close : undefined}
    >
      {children}
      {open
        ? createPortal(
            <div
              ref={panelRef}
              className={[
                "image-chip-preview",
                path && caption ? "has-caption" : caption ? "is-text" : "",
              ]
                .filter(Boolean)
                .join(" ")}
              style={{
                top: box?.top ?? 0,
                left: box?.left ?? 0,
                visibility: box && shown ? "visible" : "hidden",
              }}
              role={caption ? "tooltip" : undefined}
              aria-hidden={caption ? undefined : true}
            >
              {picture}
              {caption ? <span className="image-chip-preview-text">{caption}</span> : null}
            </div>,
            doc.body,
          )
        : null}
    </span>
  );
}

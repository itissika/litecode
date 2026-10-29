import { memo, useEffect, useRef, useState } from "react";
import { FilmSlate, ImageBroken, MusicNotes } from "@phosphor-icons/react";

import type { MediaEntry } from "../../api/workspace";
import i18n from "../../i18n";
import { formatBytes } from "../../lib/bytes";
import { formatFileTime, formatFileTimeFull } from "../../lib/fileTime";
import { fileKindFromPath } from "../../lib/fileKind";
import { HoverMediaPreview } from "./HoverMediaPreview";
import type { GalleryView } from "./galleryGroups";
import { useMediaThumb } from "./useMediaThumb";

/** Wait for a deliberate hover: a pointer crossing the grid must not play. */
const HOVER_OPEN_MS = 180;
/** Grace period so the pointer can travel from the tile into the card. */
const HOVER_CLOSE_MS = 160;

interface GalleryTileProps {
  entry: MediaEntry;
  /** Thumbnail grid or compact row. */
  view: GalleryView;
  selected: boolean;
  /** In-place rename: the caption turns into an input on this tile. */
  renaming: boolean;
  onSelect: (entry: MediaEntry) => void;
  onOpen: (entry: MediaEntry) => void;
  /** Context menu request, at viewport coordinates. */
  onMenu: (entry: MediaEntry, at: { x: number; y: number }) => void;
  onRename: (entry: MediaEntry, name: string) => void;
  onRenameCancel: () => void;
}

function extensionOf(name: string): string {
  const dot = name.lastIndexOf(".");
  return dot > 0 ? name.slice(dot + 1).toUpperCase() : "";
}

/**
 * One entry: a square tile in grid view, a compact row in list view. Images
 * paint their cached thumbnail. Video and audio keep a glyph — the server ships
 * no frame extractor — but hovering one opens a looping card, so the tile
 * itself only has to be a target.
 */
export const GalleryTile = memo(function GalleryTile({
  entry,
  view,
  selected,
  renaming,
  onSelect,
  onOpen,
  onMenu,
  onRename,
  onRenameCancel,
}: GalleryTileProps) {
  const thumb = useMediaThumb(entry);
  const ext = extensionOf(entry.name);
  const isList = view === "list";
  // A row's frame is a 26px chip; a grid tile's frame fills the square.
  const glyphSize = isList ? 14 : 22;
  const now = Date.now();

  const tileRef = useRef<HTMLButtonElement | null>(null);
  const inputRef = useRef<HTMLInputElement | null>(null);
  const [anchor, setAnchor] = useState<DOMRect | null>(null);
  const [draft, setDraft] = useState<string | null>(null);
  const openTimer = useRef<number | null>(null);
  const closeTimer = useRef<number | null>(null);
  // Enter, Escape and blur can all end one edit; only the first counts.
  const settled = useRef(false);

  // Scrolling or resizing moves the tile, so the card closes instead of
  // trailing it across the panel. Only a scroll that actually moves the tile
  // counts: capture sees every scroller in the document, and the transcript
  // gliding as tokens arrive is not this card's business.
  useEffect(() => {
    if (!anchor) return;
    const onScroll = (event: Event) => {
      const node = tileRef.current;
      const target = event.target;
      if (node && target instanceof Node && target.contains(node)) {
        setAnchor(null);
      }
    };
    const onResize = () => setAnchor(null);
    window.addEventListener("scroll", onScroll, true);
    window.addEventListener("resize", onResize);
    return () => {
      window.removeEventListener("scroll", onScroll, true);
      window.removeEventListener("resize", onResize);
    };
  }, [anchor]);

  useEffect(() => {
    return () => {
      if (openTimer.current !== null) window.clearTimeout(openTimer.current);
      if (closeTimer.current !== null) window.clearTimeout(closeTimer.current);
    };
  }, []);

  // Opening the editor preselects the stem, so typing replaces the name and
  // keeps the extension — the same reflex a file manager offers.
  useEffect(() => {
    if (!renaming) return;
    settled.current = false;
    setDraft(null);
    const node = inputRef.current;
    if (!node) return;
    node.focus();
    const dot = entry.name.lastIndexOf(".");
    node.setSelectionRange(0, dot > 0 ? dot : entry.name.length);
  }, [renaming, entry.name]);

  // Only containers the panel itself can decode get a card: an mkv or an aiff
  // would download and then fail, and its double-click goes to the OS anyway.
  const kind = fileKindFromPath(entry.path);
  const playable = kind === "audio" || kind === "video";

  const scheduleOpen = () => {
    if (!playable || renaming) return;
    if (openTimer.current !== null) window.clearTimeout(openTimer.current);
    if (closeTimer.current !== null) window.clearTimeout(closeTimer.current);
    openTimer.current = window.setTimeout(() => {
      const node = tileRef.current;
      if (node) setAnchor(node.getBoundingClientRect());
    }, HOVER_OPEN_MS);
  };

  const scheduleClose = () => {
    if (openTimer.current !== null) window.clearTimeout(openTimer.current);
    if (closeTimer.current !== null) window.clearTimeout(closeTimer.current);
    closeTimer.current = window.setTimeout(() => setAnchor(null), HOVER_CLOSE_MS);
  };

  const holdOpen = () => {
    if (closeTimer.current !== null) window.clearTimeout(closeTimer.current);
  };

  const finishRename = (name: string | null) => {
    if (settled.current) return;
    settled.current = true;
    if (name === null) onRenameCancel();
    else onRename(entry, name);
  };

  // Empty or unchanged names simply close the editor; anything else is the
  // panel's to validate and hand to the server.
  const commitRename = () => {
    const name = (draft ?? entry.name).trim();
    finishRename(!name || name === entry.name ? null : name);
  };

  const frame = (
    <span
      className={`${isList ? "gallery-row-frame" : "gallery-tile-frame"}${
        !isList && entry.kind === "image" && thumb.url ? " has-image" : ""
      }`}
    >
      {entry.kind === "image" && thumb.url ? (
        <img src={thumb.url} alt="" loading="lazy" draggable={false} />
      ) : entry.kind === "image" && thumb.failed ? (
        <span className="gallery-tile-face">
          <ImageBroken size={glyphSize} weight="thin" />
        </span>
      ) : entry.kind === "video" ? (
        <span className="gallery-tile-face gallery-tile-video">
          <FilmSlate size={glyphSize} weight="thin" />
        </span>
      ) : entry.kind === "audio" ? (
        <span className="gallery-tile-face gallery-tile-audio">
          <MusicNotes size={glyphSize} weight="thin" />
        </span>
      ) : (
        <span className="gallery-tile-face" />
      )}
      {/* The row already shows the name; a corner badge is grid-only. */}
      {!isList && (entry.kind !== "image" || thumb.failed) ? (
        <span className="gallery-tile-ext">{ext}</span>
      ) : null}
    </span>
  );

  const nameInput = (
    <input
      ref={inputRef}
      className="gallery-tile-rename"
      aria-label={i18n.t("explorer.rename")}
      spellCheck={false}
      value={draft ?? entry.name}
      onChange={(event) => setDraft(event.target.value)}
      onKeyDown={(event) => {
        if (event.key === "Enter") {
          event.preventDefault();
          commitRename();
        } else if (event.key === "Escape") {
          event.preventDefault();
          finishRename(null);
        }
      }}
      onBlur={commitRename}
    />
  );

  if (renaming) {
    return (
      <div
        className={
          (isList ? "gallery-row" : "gallery-tile") +
          " is-renaming" +
          (selected ? " is-selected" : "")
        }
        title={entry.path}
      >
        {frame}
        {nameInput}
      </div>
    );
  }

  return (
    <>
      <button
        ref={tileRef}
        type="button"
        className={
          (isList ? "gallery-row" : "gallery-tile") +
          (selected ? " is-selected" : "")
        }
        title={entry.path}
        aria-label={entry.name}
        aria-pressed={selected}
        onClick={() => onSelect(entry)}
        onDoubleClick={() => {
          setAnchor(null);
          onOpen(entry);
        }}
        onMouseEnter={scheduleOpen}
        onMouseLeave={scheduleClose}
        onContextMenu={(event) => {
          event.preventDefault();
          event.stopPropagation();
          setAnchor(null);
          onSelect(entry);
          onMenu(entry, { x: event.clientX, y: event.clientY });
        }}
        onKeyDown={(event) => {
          if (event.key === "Enter") {
            event.preventDefault();
            setAnchor(null);
            onOpen(entry);
          } else if (
            event.key === "ContextMenu" ||
            (event.shiftKey && event.key === "F10")
          ) {
            event.preventDefault();
            const node = tileRef.current;
            if (!node) return;
            const rect = node.getBoundingClientRect();
            onSelect(entry);
            onMenu(entry, { x: rect.left + 8, y: rect.top + 8 });
          }
        }}
      >
        {frame}
        {isList ? (
          <>
            <span className="gallery-row-name" title={entry.path}>
              {entry.name}
            </span>
            <span className="gallery-row-type">
              {i18n.t(`gallery.type.${entry.kind}`)}
            </span>
            <span className="gallery-row-size">
              {formatBytes(entry.size)}
            </span>
            <span
              className="gallery-row-time"
              title={formatFileTimeFull(entry.mtime, i18n.language)}
            >
              {formatFileTime(entry.mtime, now, i18n.language)}
            </span>
          </>
        ) : (
          <span className="gallery-tile-name">{entry.name}</span>
        )}
      </button>
      {anchor ? (
        <HoverMediaPreview
          key={entry.path + ":" + entry.mtime}
          entry={entry}
          anchor={anchor}
          onPointerEnter={holdOpen}
          onPointerLeave={scheduleClose}
        />
      ) : null}
    </>
  );
});

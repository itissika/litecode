import { memo } from "react";
import { FilmSlate, ImageBroken, MusicNotes } from "@phosphor-icons/react";

import type { MediaEntry } from "../../api/workspace";
import { useMediaThumb } from "./useMediaThumb";

interface GalleryTileProps {
  entry: MediaEntry;
  selected: boolean;
  onSelect: (entry: MediaEntry) => void;
  onOpen: (entry: MediaEntry) => void;
}

function extensionOf(name: string): string {
  const dot = name.lastIndexOf(".");
  return dot > 0 ? name.slice(dot + 1).toUpperCase() : "";
}

/**
 * One square tile. Images paint their cached thumbnail; video and audio keep a
 * glyph tile because the server ships no frame extractor (the system player
 * opens the original on double click).
 */
export const GalleryTile = memo(function GalleryTile({
  entry,
  selected,
  onSelect,
  onOpen,
}: GalleryTileProps) {
  const thumb = useMediaThumb(entry);
  const ext = extensionOf(entry.name);

  return (
    <button
      type="button"
      className={`gallery-tile${selected ? " is-selected" : ""}`}
      title={entry.path}
      aria-label={entry.name}
      aria-pressed={selected}
      onClick={() => onSelect(entry)}
      onDoubleClick={() => onOpen(entry)}
      onKeyDown={(event) => {
        if (event.key === "Enter") {
          event.preventDefault();
          onOpen(entry);
        }
      }}
    >
      <span className="gallery-tile-frame">
        {entry.kind === "image" && thumb.url ? (
          <img src={thumb.url} alt="" loading="lazy" draggable={false} />
        ) : entry.kind === "image" && thumb.failed ? (
          <span className="gallery-tile-face">
            <ImageBroken size={20} weight="thin" />
          </span>
        ) : entry.kind === "video" ? (
          <span className="gallery-tile-face gallery-tile-video">
            <FilmSlate size={22} weight="thin" />
          </span>
        ) : entry.kind === "audio" ? (
          <span className="gallery-tile-face gallery-tile-audio">
            <MusicNotes size={22} weight="thin" />
          </span>
        ) : (
          <span className="gallery-tile-face" />
        )}
        {entry.kind !== "image" || thumb.failed ? (
          <span className="gallery-tile-ext">{ext}</span>
        ) : null}
      </span>
      <span className="gallery-tile-name">{entry.name}</span>
    </button>
  );
});

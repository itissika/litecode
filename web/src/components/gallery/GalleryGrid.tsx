import { CaretRight } from "@phosphor-icons/react";

import type { MediaEntry } from "../../api/workspace";
import { GalleryTile } from "./GalleryTile";
import type { GallerySection, GalleryView } from "./galleryGroups";

interface GalleryGridProps {
  sections: readonly GallerySection[];
  /** Thumbnail grid or compact rows; the same choice as the folder strip. */
  view: GalleryView;
  /** Section keys the user folded away; only labelled sections fold. */
  folded: ReadonlySet<string>;
  onToggle: (key: string) => void;
  selected: string | null;
  /** Path of the tile whose caption is an input right now. */
  renaming: string | null;
  onSelect: (entry: MediaEntry) => void;
  onOpen: (entry: MediaEntry) => void;
  onMenu: (entry: MediaEntry, at: { x: number; y: number }) => void;
  onRename: (entry: MediaEntry, name: string) => void;
  onRenameCancel: () => void;
}

/**
 * Sectioned gallery. The grid is an auto-fill wall of square tiles; the list
 * paints the same entries as rows. A labelled section folds from its header:
 * the Types view is an overview first and a wall of tiles second, and a folded
 * section mounts nothing, so it fetches no thumbnails either. Totals stay
 * visible while folded and while capped by paging.
 */
export function GalleryGrid({
  sections,
  view,
  folded,
  onToggle,
  selected,
  renaming,
  onSelect,
  onOpen,
  onMenu,
  onRename,
  onRenameCancel,
}: GalleryGridProps) {
  return (
    <div className="gallery-sections">
      {sections.map((section) => {
        const isFolded = Boolean(section.label) && folded.has(section.key);
        return (
          <section key={section.key} className="gallery-section">
            {section.label ? (
              <button
                type="button"
                className={"gallery-section-head" + (isFolded ? " is-folded" : "")}
                aria-expanded={!isFolded}
                aria-controls={"gallery-section-" + section.key}
                onClick={() => onToggle(section.key)}
              >
                <CaretRight
                  size={9}
                  weight="bold"
                  className="gallery-section-caret"
                />
                <span className="gallery-section-label">{section.label}</span>
                <span className="gallery-section-count">{section.total}</span>
              </button>
            ) : null}
            <div
              className={view === "list" ? "gallery-list" : "gallery-grid"}
              id={"gallery-section-" + section.key}
            >
              {isFolded
                ? null
                : section.entries.map((entry) => (
                    <GalleryTile
                      key={entry.path}
                      entry={entry}
                      view={view}
                      selected={selected === entry.path}
                      renaming={renaming === entry.path}
                      onSelect={onSelect}
                      onOpen={onOpen}
                      onMenu={onMenu}
                      onRename={onRename}
                      onRenameCancel={onRenameCancel}
                    />
                  ))}
            </div>
          </section>
        );
      })}
    </div>
  );
}

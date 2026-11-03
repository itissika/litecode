import { CaretRight } from "@phosphor-icons/react";
import type { KeyboardEvent } from "react";

import type { MediaEntry } from "../../api/workspace";
import { GalleryTile } from "./GalleryTile";
import type { GallerySection, GalleryView } from "./galleryGroups";

interface GalleryGridProps {
  sections: readonly GallerySection[];
  /** Accessible name for sections without their own label. */
  ariaLabel: string;
  /** Thumbnail grid or compact rows; the same choice as the folder strip. */
  view: GalleryView;
  /** Section keys the user folded away; only labelled sections fold. */
  folded: ReadonlySet<string>;
  onToggle: (key: string) => void;
  selected: ReadonlySet<string>;
  /** Roving focus target; exactly one tile is tabbable. */
  active: string | null;
  /** Path of the tile whose caption is an input right now. */
  renaming: string | null;
  onSelect: (
    entry: MediaEntry,
    opts?: { additive?: boolean; range?: boolean },
  ) => void;
  onActivate: (entry: MediaEntry) => void;
  onKeyDown: (entry: MediaEntry, event: KeyboardEvent<HTMLElement>) => void;
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
  ariaLabel,
  view,
  folded,
  onToggle,
  selected,
  active,
  renaming,
  onSelect,
  onActivate,
  onKeyDown,
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
              role="listbox"
              aria-label={section.label ?? ariaLabel}
              aria-multiselectable="true"
            >
              {isFolded
                ? null
                : section.entries.map((entry) => (
                    <GalleryTile
                      key={entry.path}
                      entry={entry}
                      view={view}
                      selected={selected.has(entry.path)}
                      active={active === entry.path}
                      renaming={renaming === entry.path}
                      onSelect={onSelect}
                      onActivate={onActivate}
                      onKeyDown={onKeyDown}
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

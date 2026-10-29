import type { MediaEntry } from "../../api/workspace";
import { GalleryTile } from "./GalleryTile";
import type { GallerySection } from "./galleryGroups";

interface GalleryGridProps {
  sections: readonly GallerySection[];
  selected: string | null;
  onSelect: (entry: MediaEntry) => void;
  onOpen: (entry: MediaEntry) => void;
}

/** Sectioned, auto-fill square grid. Section totals stay visible while capped. */
export function GalleryGrid({
  sections,
  selected,
  onSelect,
  onOpen,
}: GalleryGridProps) {
  return (
    <div className="gallery-sections">
      {sections.map((section) => (
        <section key={section.key} className="gallery-section">
          {section.label ? (
            <header className="gallery-section-head">
              <span>{section.label}</span>
              <span className="gallery-section-count">{section.total}</span>
            </header>
          ) : null}
          <div className="gallery-grid">
            {section.entries.map((entry) => (
              <GalleryTile
                key={entry.path}
                entry={entry}
                selected={selected === entry.path}
                onSelect={onSelect}
                onOpen={onOpen}
              />
            ))}
          </div>
        </section>
      ))}
    </div>
  );
}

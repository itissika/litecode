import type { MediaEntry, MediaKind } from "../../api/workspace";

/** Grouping model for the Gallery panel. Pure data, no React. */

export type GalleryMode = "all" | "folder" | "kind";

/** How the panel paints entries: square thumbnails or compact rows. */
export type GalleryView = "grid" | "list";

export type GalleryKind = MediaKind;

export const MEDIA_KIND_ORDER: readonly MediaKind[] = [
  "image",
  "video",
  "audio",
];

export interface GallerySection {
  key: string;
  label?: string;
  entries: MediaEntry[];
  /** Entries in the source section, before incremental capping. */
  total: number;
}

export interface GalleryFolder {
  /** Workspace-relative path, / separated. */
  path: string;
  name: string;
  parent: string;
  /** Media files at or below this folder. */
  count: number;
  /** Media files directly inside this folder. */
  direct: number;
}

function byName(a: { name: string }, b: { name: string }): number {
  return a.name.localeCompare(b.name, undefined, {
    numeric: true,
    sensitivity: "base",
  });
}

/**
 * Every folder that holds media, with a recursive count. Root is implicit:
 * entries whose folder is empty live at the workspace root.
 */
export function buildFolderIndex(
  entries: readonly MediaEntry[],
): Map<string, GalleryFolder> {
  const index = new Map<string, GalleryFolder>();
  const ensure = (path: string): GalleryFolder => {
    const existing = index.get(path);
    if (existing) return existing;
    const slash = path.lastIndexOf("/");
    const folder: GalleryFolder = {
      path,
      name: slash >= 0 ? path.slice(slash + 1) : path,
      parent: slash >= 0 ? path.slice(0, slash) : "",
      count: 0,
      direct: 0,
    };
    index.set(path, folder);
    return folder;
  };

  for (const entry of entries) {
    let dir = entry.folder;
    while (dir) {
      const folder = ensure(dir);
      folder.count += 1;
      dir = folder.parent;
    }
    if (entry.folder) ensure(entry.folder).direct += 1;
  }
  return index;
}

/** Immediate child folders of `parent` ("" = workspace root), sorted by name. */
export function childFolders(
  index: Map<string, GalleryFolder>,
  parent: string,
): GalleryFolder[] {
  const out: GalleryFolder[] = [];
  for (const folder of index.values()) {
    if (folder.parent === parent) out.push(folder);
  }
  return out.sort(byName);
}

/** Media directly inside one folder, sorted by name. */
export function folderEntries(
  entries: readonly MediaEntry[],
  folder: string,
): MediaEntry[] {
  return entries.filter((entry) => entry.folder === folder).sort(byName);
}

export function allSection(entries: readonly MediaEntry[]): GallerySection[] {
  if (entries.length === 0) return [];
  const sorted = [...entries].sort(byName);
  return [{ key: "all", entries: sorted, total: sorted.length }];
}

/** Image / video / audio sections, empty kinds omitted, in a fixed order. */
export function kindSections(
  entries: readonly MediaEntry[],
  label: (kind: MediaKind) => string,
): GallerySection[] {
  const sections: GallerySection[] = [];
  for (const kind of MEDIA_KIND_ORDER) {
    const items = entries.filter((entry) => entry.kind === kind);
    if (items.length === 0) continue;
    items.sort(byName);
    sections.push({
      key: kind,
      label: label(kind),
      entries: items,
      total: items.length,
    });
  }
  return sections;
}

/** Keep the first `limit` tiles of each section for incremental rendering. */
export function capSections(
  sections: readonly GallerySection[],
  limit: number,
): GallerySection[] {
  if (limit < 0) return [];
  return sections.map((section) => ({
    ...section,
    entries: section.entries.slice(0, limit),
    total: section.total,
  }));
}

export function sectionsHaveMore(
  sections: readonly GallerySection[],
  limit: number,
): boolean {
  return sections.some((section) => section.entries.length > limit);
}

/** Folder breadcrumb segments, root first: "" then "a", "a/b", ... */
export function folderTrail(folder: string): string[] {
  if (!folder) return [""];
  const parts = folder.split("/").filter(Boolean);
  const trail = [""];
  let current = "";
  for (const part of parts) {
    current = current ? `${current}/${part}` : part;
    trail.push(current);
  }
  return trail;
}

export function folderLabel(folder: string): string {
  if (!folder) return "";
  const slash = folder.lastIndexOf("/");
  return slash >= 0 ? folder.slice(slash + 1) : folder;
}

export function parentFolder(folder: string): string {
  const slash = folder.lastIndexOf("/");
  return slash >= 0 ? folder.slice(0, slash) : "";
}

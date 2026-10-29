import { describe, expect, it } from "vitest";

import type { MediaEntry, MediaKind } from "../../api/workspace";
import {
  allSection,
  buildFolderIndex,
  capSections,
  childFolders,
  folderEntries,
  folderLabel,
  folderTrail,
  kindSections,
  parentFolder,
  sectionsHaveMore,
} from "./galleryGroups";

function entry(path: string, kind: MediaKind = "image"): MediaEntry {
  const slash = path.lastIndexOf("/");
  return {
    path,
    name: path.slice(slash + 1),
    folder: slash >= 0 ? path.slice(0, slash) : "",
    kind,
    size: 10,
    mtime: 1,
  };
}

describe("galleryGroups", () => {
  const entries = [
    entry("root.png"),
    entry("a/shot.png"),
    entry("a/b/reel.mp4", "video"),
    entry("a/b/theme.mp3", "audio"),
    entry("a/b/zz.png"),
  ];

  it("counts folders recursively and keeps direct totals", () => {
    const index = buildFolderIndex(entries);
    expect(index.get("a")?.count).toBe(4);
    expect(index.get("a")?.direct).toBe(1);
    expect(index.get("a/b")?.count).toBe(3);
    expect(index.get("a/b")?.direct).toBe(3);
    expect(index.has("")).toBe(false);

    expect(childFolders(index, "").map((f) => f.path)).toEqual(["a"]);
    expect(childFolders(index, "a").map((f) => f.path)).toEqual(["a/b"]);
    expect(childFolders(index, "a/b")).toEqual([]);
  });

  it("lists direct folder entries by name", () => {
    const direct = folderEntries(entries, "a/b");
    expect(direct.map((item) => item.name)).toEqual([
      "reel.mp4",
      "theme.mp3",
      "zz.png",
    ]);
    expect(folderEntries(entries, "")).toHaveLength(1);
  });

  it("builds fixed-order kind sections and omits empty kinds", () => {
    const sections = kindSections(entries, (kind) => kind.toUpperCase());
    expect(sections.map((section) => section.key)).toEqual([
      "image",
      "video",
      "audio",
    ]);
    expect(sections[0].label).toBe("IMAGE");
    expect(sections[0].total).toBe(3);
    expect(sections[1].entries.map((item) => item.name)).toEqual(["reel.mp4"]);

    const onlyImages = kindSections([entry("x.png")], (kind) => kind);
    expect(onlyImages.map((section) => section.key)).toEqual(["image"]);
  });

  it("caps tiles per section without losing totals", () => {
    const sections = kindSections(entries, (kind) => kind);
    const capped = capSections(sections, 1);
    expect(capped.map((section) => section.entries.length)).toEqual([1, 1, 1]);
    expect(capped.map((section) => section.total)).toEqual([3, 1, 1]);
    expect(sectionsHaveMore(sections, 1)).toBe(true);
    expect(sectionsHaveMore(sections, 3)).toBe(false);

    const all = capSections(allSection(entries), 2);
    expect(all[0].entries).toHaveLength(2);
    expect(all[0].total).toBe(5);
  });

  it("keeps all-section order by name", () => {
    const section = allSection(entries)[0];
    expect(section.entries.map((item) => item.path)).toEqual([
      "a/b/reel.mp4",
      "root.png",
      "a/shot.png",
      "a/b/theme.mp3",
      "a/b/zz.png",
    ]);
  });

  it("splits folder trails", () => {
    expect(folderTrail("")).toEqual([""]);
    expect(folderTrail("a")).toEqual(["", "a"]);
    expect(folderTrail("a/b/c")).toEqual(["", "a", "a/b", "a/b/c"]);
    expect(folderLabel("a/b/c")).toBe("c");
    expect(parentFolder("a/b/c")).toBe("a/b");
    expect(parentFolder("a")).toBe("");
  });
});

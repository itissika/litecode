import { describe, expect, it } from "vitest";

import type { TreeEntry } from "../api/workspace";
import { flattenVisibleRows, visibleEntryPaths } from "./fileTreeVisible";

const dir = (path: string, name: string): TreeEntry => ({
  path,
  name,
  kind: "dir",
});
const file = (path: string, name: string): TreeEntry => ({
  path,
  name,
  kind: "file",
});

describe("flattenVisibleRows", () => {
  it("returns an empty list for an empty tree", () => {
    expect(flattenVisibleRows({}, new Set())).toEqual([]);
  });

  it("lists root entries without expanding", () => {
    const children = {
      "": [dir("src", "src"), file("a.txt", "a.txt")],
      src: [file("src/main.rs", "main.rs")],
    };
    const rows = flattenVisibleRows(children, new Set());
    expect(visibleEntryPaths(rows)).toEqual(["src", "a.txt"]);
    expect(rows.map((r) => (r.type === "entry" ? r.depth : null))).toEqual([
      0, 0,
    ]);
  });

  it("nests expanded directory children after the directory", () => {
    const children = {
      "": [dir("src", "src"), file("README.md", "README.md")],
      src: [dir("src/lib", "lib"), file("src/main.rs", "main.rs")],
      "src/lib": [file("src/lib/mod.rs", "mod.rs")],
    };
    const rows = flattenVisibleRows(
      children,
      new Set(["src", "src/lib"]),
    );
    expect(visibleEntryPaths(rows)).toEqual([
      "src",
      "src/lib",
      "src/lib/mod.rs",
      "src/main.rs",
      "README.md",
    ]);
    expect(
      rows.filter((r) => r.type === "entry").map((r) => r.depth),
    ).toEqual([0, 1, 2, 1, 0]);
  });

  it("does not paint children of a collapsed directory", () => {
    const children = {
      "": [dir("src", "src")],
      src: [file("src/main.rs", "main.rs")],
    };
    const rows = flattenVisibleRows(children, new Set());
    expect(visibleEntryPaths(rows)).toEqual(["src"]);
  });

  it("inserts a root ghost before root children", () => {
    const children = { "": [file("a.txt", "a.txt")] };
    const rows = flattenVisibleRows(children, new Set(), {
      parent: "",
      kind: "newFile",
    });
    expect(rows[0]).toEqual({
      type: "ghost",
      parent: "",
      depth: 0,
      kind: "newFile",
    });
    expect(visibleEntryPaths(rows)).toEqual(["a.txt"]);
  });

  it("inserts a nested ghost before that directory's children", () => {
    const children = {
      "": [dir("src", "src")],
      src: [file("src/main.rs", "main.rs")],
    };
    const rows = flattenVisibleRows(children, new Set(["src"]), {
      parent: "src",
      kind: "newFolder",
    });
    expect(visibleEntryPaths(rows)).toEqual(["src", "src/main.rs"]);
    const ghost = rows.find((r) => r.type === "ghost");
    expect(ghost).toEqual({
      type: "ghost",
      parent: "src",
      depth: 1,
      kind: "newFolder",
    });
    expect(rows[1]?.type).toBe("ghost");
    expect(rows[2]?.type === "entry" && rows[2].entry.path).toBe("src/main.rs");
  });
});

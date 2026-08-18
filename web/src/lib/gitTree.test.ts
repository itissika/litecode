import { describe, expect, it } from "vitest";

import type { GitFile } from "../api/workspace";
import { buildGitTree, descendantFiles, visibleFileIds } from "./gitTree";

function file(path: string): GitFile {
  return { path, status: "M", untracked: false };
}

describe("buildGitTree", () => {
  it("nests files under folders and sorts dirs first", () => {
    const tree = buildGitTree([
      file("z.ts"),
      file("src/a.ts"),
      file("src/lib/b.ts"),
    ]);
    expect(tree.map((n) => n.name)).toEqual(["src", "z.ts"]);
    const src = tree[0];
    expect(src?.kind).toBe("dir");
    if (src?.kind !== "dir") return;
    expect(src.children.map((n) => n.name)).toEqual(["lib", "a.ts"]);
  });

  it("lists descendant files for a folder action", () => {
    const tree = buildGitTree([file("src/a.ts"), file("src/b.ts")]);
    expect(descendantFiles(tree[0]!).map((f) => f.path)).toEqual([
      "src/a.ts",
      "src/b.ts",
    ]);
  });

  it("skips collapsed directories in visible file ids", () => {
    const tree = buildGitTree([file("src/a.ts"), file("root.ts")]);
    const ids = visibleFileIds(tree, "changes", new Set(["src"]), (s, p) => `${s}:${p}`);
    expect(ids).toEqual(["changes:root.ts"]);
  });
});

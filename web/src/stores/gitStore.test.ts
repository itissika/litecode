import { describe, expect, it } from "vitest";

import { gitRowId, isGitMetaPath, parseGitRowId, selectedPaths, watchPathsAffectGitWorktree } from "./gitStore";

describe("git row ids", () => {
  it("round-trips section and path", () => {
    const id = gitRowId("changes", "src/foo.rs");
    expect(parseGitRowId(id)).toEqual({ section: "changes", path: "src/foo.rs" });
  });

  it("filters selected paths by section", () => {
    const selected = new Set([
      gitRowId("staged", "a.ts"),
      gitRowId("changes", "b.ts"),
      gitRowId("changes", "c.ts"),
    ]);
    expect(selectedPaths(selected, "staged")).toEqual(["a.ts"]);
    expect(selectedPaths(selected, "changes")).toEqual(["b.ts", "c.ts"]);
  });
});

describe("git watch path filter", () => {
  it("treats .git internals as metadata", () => {
    expect(isGitMetaPath(".git/index")).toBe(true);
    expect(isGitMetaPath(".git/HEAD")).toBe(true);
    expect(isGitMetaPath("nested/.git/config")).toBe(true);
    expect(isGitMetaPath("src/main.rs")).toBe(false);
  });

  it("ignores refresh when every changed path is git metadata", () => {
    expect(watchPathsAffectGitWorktree([".git/index", ".git/HEAD"])).toBe(false);
    expect(watchPathsAffectGitWorktree([".git/index", "src/a.ts"])).toBe(true);
    expect(watchPathsAffectGitWorktree([])).toBe(true);
  });
});

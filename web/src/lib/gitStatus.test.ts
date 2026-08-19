import { describe, expect, it } from "vitest";

import type { GitFile, GitStatus } from "../api/workspace";
import {
  gitChangedDirs,
  gitFileLetters,
  gitStatusColor,
  gitStatusLabel,
  treeGitLetter,
} from "./gitStatus";

function file(path: string, status: string, untracked = false): GitFile {
  return { path, status, untracked };
}

function status(staged: GitFile[], changes: GitFile[]): GitStatus {
  return {
    is_repo: true,
    branch: "main",
    upstream_ahead: 0,
    upstream_behind: 0,
    staged,
    changes,
  };
}

describe("gitStatusLabel", () => {
  it("maps untracked and staged additions to N", () => {
    expect(gitStatusLabel(file("a.ts", "?", true))).toBe("N");
    expect(gitStatusLabel(file("a.ts", "A"))).toBe("N");
  });

  it("keeps the other letters", () => {
    expect(gitStatusLabel(file("a.ts", "M"))).toBe("M");
    expect(gitStatusLabel(file("a.ts", "D"))).toBe("D");
    expect(gitStatusLabel(file("a.ts", "R"))).toBe("R");
    expect(gitStatusLabel(file("a.ts", "C"))).toBe("C");
    expect(gitStatusLabel(file("a.ts", "U"))).toBe("U");
  });
});

describe("gitStatusColor", () => {
  it("matches the SCM panel palette", () => {
    expect(gitStatusColor("N")).toBe("text-(--_dk-emerald-500)");
    expect(gitStatusColor("M")).toBe("text-(--_dk-amber-500)");
    expect(gitStatusColor("D")).toBe("text-(--_dk-red-500)");
    expect(gitStatusColor("U")).toBe("text-(--_dk-cat-purple)");
    expect(gitStatusColor("R")).toBe("text-(--_dk-cat-blue)");
    expect(gitStatusColor("C")).toBe("text-(--_dk-cat-cyan)");
  });
});

describe("treeGitLetter", () => {
  it("only surfaces 增删改 (N/M/D)", () => {
    expect(treeGitLetter(file("a", "?", true))).toBe("N");
    expect(treeGitLetter(file("a", "A"))).toBe("N");
    expect(treeGitLetter(file("a", "M"))).toBe("M");
    expect(treeGitLetter(file("a", "D"))).toBe("D");
    expect(treeGitLetter(file("a", "R"))).toBeNull();
    expect(treeGitLetter(file("a", "C"))).toBeNull();
    expect(treeGitLetter(file("a", "U"))).toBeNull();
  });
});

describe("gitFileLetters", () => {
  it("merges staged and worktree, strongest wins", () => {
    const s = status([file("n.ts", "A")], [file("m.ts", "M")]);
    s.staged.push(file("both.ts", "A"));
    s.changes.push(file("both.ts", "M"));
    const letters = gitFileLetters(s);
    expect(letters.get("n.ts")).toBe("N");
    expect(letters.get("m.ts")).toBe("M");
    expect(letters.get("both.ts")).toBe("N"); // N beats M
  });

  it("ignores rename/copy/conflict", () => {
    const s = status([file("r.ts", "R")], [file("c.ts", "C"), file("u.ts", "U")]);
    expect(gitFileLetters(s).size).toBe(0);
  });
});

describe("gitChangedDirs", () => {
  it("collects every ancestor of a changed file", () => {
    const s = status([], [file("src/app/main.ts", "M")]);
    const dirs = gitChangedDirs(s);
    expect(dirs.has("src")).toBe(true);
    expect(dirs.has("src/app")).toBe(true);
    expect(dirs.has("")).toBe(false);
  });

  it("includes deleted files' ancestors even though the file is gone", () => {
    const s = status([], [file("gone/lib.ts", "D")]);
    expect(gitChangedDirs(s).has("gone")).toBe(true);
  });
});

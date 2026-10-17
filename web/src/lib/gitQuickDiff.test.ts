import { describe, expect, it } from "vitest";

import {
  classifyChange,
  decorationRange,
  dropOverlappingSecondary,
  hunkAtLine,
  hunksFromChanges,
  originalsForPath,
  peekAfterLine,
  peekHeightPx,
  stagedOriginalText,
  type DiffHunk,
  type LineChange,
  type QuickDiffFile,
} from "./gitQuickDiff";

function change(
  originalStart: number,
  originalEnd: number,
  modifiedStart: number,
  modifiedEnd: number,
): LineChange {
  return {
    originalStartLineNumber: originalStart,
    originalEndLineNumber: originalEnd,
    modifiedStartLineNumber: modifiedStart,
    modifiedEndLineNumber: modifiedEnd,
  };
}

function file(partial: Partial<QuickDiffFile> & { path: string }): QuickDiffFile {
  return {
    status: "M",
    untracked: false,
    ...partial,
  };
}

describe("classifyChange", () => {
  it("splits inserts, deletes, and edits", () => {
    expect(classifyChange(change(3, 0, 4, 6))).toBe("add");
    expect(classifyChange(change(4, 6, 3, 0))).toBe("delete");
    expect(classifyChange(change(2, 4, 2, 5))).toBe("modify");
  });
});

describe("dropOverlappingSecondary", () => {
  it("keeps a staged hunk that does not meet a worktree hunk", () => {
    const hunks: DiffHunk[] = [
      ...hunksFromChanges("worktree", [change(1, 2, 1, 2)]),
      ...hunksFromChanges("staged", [change(10, 12, 10, 12)]),
    ];
    expect(dropOverlappingSecondary(hunks)).toHaveLength(2);
  });

  it("drops a staged hunk that shares lines with a worktree hunk", () => {
    const hunks: DiffHunk[] = [
      ...hunksFromChanges("worktree", [change(4, 6, 4, 6)]),
      ...hunksFromChanges("staged", [change(2, 4, 2, 4)]),
    ];
    const kept = dropOverlappingSecondary(hunks);
    expect(kept.map((hunk) => hunk.baseline)).toEqual(["worktree"]);
  });

  it("drops a whole-file staged add when any worktree hunk exists", () => {
    const hunks: DiffHunk[] = [
      ...hunksFromChanges("worktree", [change(5, 0, 5, 6)]),
      ...hunksFromChanges("staged", [change(0, 0, 1, 100)]),
    ];
    expect(dropOverlappingSecondary(hunks)).toHaveLength(1);
  });
});

describe("decorationRange", () => {
  it("pins a deletion to the anchor line", () => {
    const [hunk] = hunksFromChanges("worktree", [change(4, 6, 3, 0)]);
    expect(decorationRange(hunk, 20)).toMatchObject({
      startLineNumber: 3,
      endLineNumber: 3,
    });
  });

  it("places a deletion at the top of the file on line 1", () => {
    const [hunk] = hunksFromChanges("worktree", [change(1, 2, 0, 0)]);
    expect(decorationRange(hunk, 10).startLineNumber).toBe(1);
    expect(hunkAtLine([hunk], 1)).toBe(hunk);
  });

  it("clamps to the model line count", () => {
    const [hunk] = hunksFromChanges("worktree", [change(0, 0, 8, 12)]);
    expect(decorationRange(hunk, 9)).toMatchObject({
      startLineNumber: 8,
      endLineNumber: 9,
    });
  });
});

describe("peek placement", () => {
  it("opens after the last modified line and caps the height", () => {
    const [hunk] = hunksFromChanges("worktree", [change(2, 4, 2, 6)]);
    expect(peekAfterLine(hunk)).toBe(6);
    const height = peekHeightPx(hunk, 20, 300);
    expect(height).toBe(100);
  });
});

describe("originalsForPath", () => {
  it("skips untracked files and conflicts", () => {
    const status = {
      is_repo: true,
      changes: [file({ path: "new.ts", status: "?", untracked: true })],
      staged: [file({ path: "conflict.ts", status: "U" })],
    };
    expect(originalsForPath(status, "new.ts")).toEqual({
      worktree: null,
      staged: null,
    });
    expect(originalsForPath(status, "conflict.ts")).toEqual({
      worktree: null,
      staged: null,
    });
  });

  it("loads the index for a worktree edit and HEAD for a staged edit", () => {
    const status = {
      is_repo: true,
      changes: [file({ path: "a.ts" })],
      staged: [file({ path: "b.ts", status: "R", orig_path: "old.ts" })],
    };
    expect(originalsForPath(status, "a.ts").worktree?.path).toBe("a.ts");
    expect(originalsForPath(status, "a.ts").staged).toBeNull();
    expect(originalsForPath(status, "b.ts").staged?.orig_path).toBe("old.ts");
  });

  it("treats a staged addition with no HEAD blob as an empty original", () => {
    const added = file({ path: "c.ts", status: "A" });
    expect(stagedOriginalText(added, null)).toBe("");
    expect(stagedOriginalText(file({ path: "a.ts" }), null)).toBeNull();
    expect(stagedOriginalText(file({ path: "a.ts" }), "old\n")).toBe("old\n");
  });
});

import { describe, expect, it } from "vitest";

import { moveFolderId, reorderFolderIds } from "./sessionFolders";

const ids = ["a", "b", "c", "d"];

describe("reorderFolderIds", () => {
  it("drops before and after a target", () => {
    expect(reorderFolderIds(ids, "d", "b", "before")).toEqual([
      "a",
      "d",
      "b",
      "c",
    ]);
    expect(reorderFolderIds(ids, "a", "c", "after")).toEqual([
      "b",
      "c",
      "a",
      "d",
    ]);
    expect(reorderFolderIds(ids, "a", "b", "after")).toEqual([
      "b",
      "a",
      "c",
      "d",
    ]);
  });

  it("returns null for a no-op or an unknown id", () => {
    expect(reorderFolderIds(ids, "a", "b", "before")).toBeNull();
    expect(reorderFolderIds(ids, "missing", "b", "before")).toBeNull();
    expect(reorderFolderIds(ids, "a", "missing", "after")).toBeNull();
  });
});

describe("moveFolderId", () => {
  it("swaps one slot and stops at the edges", () => {
    expect(moveFolderId(ids, "b", -1)).toEqual(["b", "a", "c", "d"]);
    expect(moveFolderId(ids, "c", 1)).toEqual(["a", "b", "d", "c"]);
    expect(moveFolderId(ids, "a", -1)).toBeNull();
    expect(moveFolderId(ids, "d", 1)).toBeNull();
    expect(moveFolderId(ids, "missing", 1)).toBeNull();
  });
});

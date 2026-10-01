import { beforeEach, describe, expect, it, vi } from "vitest";

import { fileCandidates, filterSymbols, lockedQuery, phaseForQuery } from "./suggestions";

const fetchMentionPaths = vi.hoisted(() => vi.fn());

vi.mock("../../api/workspace", () => ({
  fetchMentionPaths,
  fetchSymbols: vi.fn(),
}));

describe("file suggestion phase", () => {
  beforeEach(() => {
    fetchMentionPaths.mockReset();
  });

  it("asks the path index with the cleaned query", async () => {
    fetchMentionPaths.mockResolvedValue([{ path: "src/A.rs", file: true }]);
    await expect(fileCandidates("A*.rs")).resolves.toEqual([{ path: "src/A.rs", file: true }]);
    expect(fetchMentionPaths).toHaveBeenCalledWith("A.rs", undefined);
  });

  it("drops a stale response when a later keystroke is in flight", async () => {
    let resolveFirst: (value: { path: string; file: boolean }[]) => void = () => undefined;
    fetchMentionPaths.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          resolveFirst = resolve;
        }),
    );
    fetchMentionPaths.mockResolvedValueOnce([{ path: "src/b.rs", file: true }]);
    const first = fileCandidates("a");
    const second = fileCandidates("b");
    resolveFirst([{ path: "src/a.rs", file: true }]);
    await expect(second).resolves.toEqual([{ path: "src/b.rs", file: true }]);
    await expect(first).resolves.toEqual([]);
  });

  const first = { path: "src/a/manager.rs", name: "manager.rs" };
  const second = { path: "src/b/manager.rs", name: "manager.rs" };

  it("stays on files until a path is locked", () => {
    expect(phaseForQuery("mang", null)).toEqual({ mode: "file", filter: "mang" });
    expect(lockedQuery(first.name)).toBe("/manager.rs#");
  });

  it("locks the highlighted path when two files share a name", () => {
    expect(phaseForQuery("manager.rs#tes", second)).toEqual({
      mode: "symbol",
      filter: "tes",
      locked: second,
    });
    const locked = phaseForQuery("manager.rs#tes", first);
    expect(locked.mode === "symbol" && locked.locked).toEqual(first);
  });

  it("unlocks when the hash is deleted", () => {
    expect(phaseForQuery("manager.rs", second)).toEqual({ mode: "file", filter: "manager.rs" });
    expect(phaseForQuery("manager", second).mode).toBe("file");
  });

  it("filters symbols by chain or name", () => {
    const symbols = [
      { chain: "impl Store › fn save", kind: "function", name: "save", start_line: 1, end_line: 3, summary: "" },
      { chain: "fn test_run", kind: "function", name: "test_run", start_line: 4, end_line: 8, summary: "" },
    ];
    expect(filterSymbols(symbols, "tes").map((item) => item.name)).toEqual(["test_run"]);
    expect(filterSymbols(symbols, "").map((item) => item.name)).toEqual(["save", "test_run"]);
  });
});

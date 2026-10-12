import { describe, expect, it } from "vitest";

import { splitSyncPct } from "./syncPct";

describe("splitSyncPct", () => {
  it("parses pct and optional label, returning the rest", () => {
    expect(splitSyncPct("sync_pct: 87%\nhits below")).toEqual({
      sync: { pct: 87 },
      rest: "hits below",
    });
    expect(splitSyncPct("sync_pct: 0% (syncing)\nbody")).toEqual({
      sync: { pct: 0, label: "syncing" },
      rest: "body",
    });
    expect(splitSyncPct("sync_pct: 100%\n")).toEqual({
      sync: { pct: 100 },
      rest: "",
    });
  });

  it("omits when the field is missing (never invents 100%)", () => {
    expect(splitSyncPct("No matching anything.")).toEqual({
      sync: null,
      rest: "No matching anything.",
    });
    expect(splitSyncPct("")).toEqual({ sync: null, rest: "" });
  });

  it("rejects out-of-range or malformed lines without inventing a value", () => {
    expect(splitSyncPct("sync_pct: 101%\nbody").sync).toBeNull();
    expect(splitSyncPct("sync_pct: -1%\nbody").sync).toBeNull();
    expect(splitSyncPct("sync_pct: ready\nbody").sync).toBeNull();
    expect(splitSyncPct("prefix sync_pct: 50%\nbody").sync).toBeNull();
  });
});

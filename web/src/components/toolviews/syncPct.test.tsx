import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { SyncPctHeader } from "./SyncPctHeader";
import { parseSyncPctLine, splitSyncPct } from "./syncPct";

afterEach(() => cleanup());

describe("WP-138 sync_pct v2 — accepted first lines", () => {
  it.each([
    [
      "sync_pct: 98% (refreshing, embedding) chunks 9800/10000",
      { kind: "pct", pct: 98, label: "refreshing, embedding", chunks: { done: 9800, total: 10000 } },
    ],
    ["sync_pct: 100% (ready) chunks 5/5", { kind: "pct", pct: 100, label: "ready", chunks: { done: 5, total: 5 } }],
    ["sync_pct: 0%", { kind: "pct", pct: 0 }],
    ["sync_pct: 42% (ready)", { kind: "pct", pct: 42, label: "ready" }],
    ["sync_pct: 7% chunks 7/100", { kind: "pct", pct: 7, chunks: { done: 7, total: 100 } }],
    ["sync_pct: 3% chunks 9/2", { kind: "pct", pct: 3, chunks: { done: 9, total: 2 } }], // Q4: no done≤total check
    ["sync_pct: scanning (refreshing, scanning)", { kind: "scanning", label: "refreshing, scanning" }],
    ["sync_pct: scanning", { kind: "scanning" }],
    ["sync_pct: failed (model_missing)", { kind: "failed", label: "model_missing" }],
    ["sync_pct: unknown (needs_rebuild)", { kind: "unknown", label: "needs_rebuild" }],
    ["sync_pct: unknown", { kind: "unknown" }],
    ["sync_pct: 87% (a b_c-d.e, f)", { kind: "pct", pct: 87, label: "a b_c-d.e, f" }], // Q3 charset
  ])("%s", (line, expected) => {
    expect(parseSyncPctLine(line)).toEqual(expected);
  });
});

describe("WP-138 sync_pct v2 — rejected shapes (Q2)", () => {
  it.each([
    "sync_pct: 87", // v1-ish bare number
    "sync_pct: 101%",
    "sync_pct: -1%",
    "sync_pct: 1000%",
    "sync_pct: Scanning",
    "sync_pct: FAILED (x)",
    "SYNC_PCT: 5%",
    "sync_pct: syncing",
    "sync_pct: failed chunks 1/2",
    "sync_pct: scanning (x) chunks 1/2",
    "sync_pct: unknown chunks 0/0",
    "sync_pct:  5%",
    "sync_pct: 5%  (x)",
    "sync_pct: 5% (x) ",
    "sync_pct: 5% (x) extra",
    "sync_pct: 5% ()", // Q3 empty label
    "sync_pct: 5% (a(b))",
    "sync_pct: 5% chunks -1/2",
    "sync_pct: 5% chunks 1.5/2",
    "sync_label: syncing",
    " sync_pct: 5%",
  ])("%s", (line) => {
    expect(parseSyncPctLine(line)).toBeNull();
    const raw = `${line}\nbody`;
    // Q1: not recognised → no header, line stays in body untouched.
    expect(splitSyncPct(raw)).toEqual({ sync: null, rest: raw });
  });
});

describe("splitSyncPct rest handling", () => {
  it("strips a recognised first line and keeps the body (incl. empty hits)", () => {
    expect(splitSyncPct("sync_pct: unknown (needs_rebuild)\n没有匹配")).toEqual({
      sync: { kind: "unknown", label: "needs_rebuild" },
      rest: "没有匹配",
    });
    expect(splitSyncPct("sync_pct: 5%\r\nline")).toEqual({
      sync: { kind: "pct", pct: 5 },
      rest: "line",
    });
    expect(splitSyncPct("sync_pct: scanning").rest).toBe("");
    expect(splitSyncPct("")).toEqual({ sync: null, rest: "" });
    expect(splitSyncPct("plain\nsync_pct: 5%").sync).toBeNull();
  });
});

describe("SyncPctHeader renders the four values", () => {
  it("pct with chunks + label", () => {
    render(<SyncPctHeader sync={splitSyncPct("sync_pct: 98% (refreshing, embedding) chunks 9800/10000").sync} />);
    expect(screen.getByTestId("tool-sync-pct").dataset.kind).toBe("pct");
    expect(screen.getByTestId("tool-sync-pct-value").textContent).toBe("98%");
    expect(screen.getByTestId("tool-sync-pct-chunks").textContent).toBe("9800/10000 chunks");
    expect(screen.getByTestId("tool-sync-pct-label").textContent).toBe("refreshing, embedding");
  });
  it.each(["scanning", "failed", "unknown"] as const)("%s word, no chunks", (word) => {
    render(<SyncPctHeader sync={splitSyncPct(`sync_pct: ${word} (x)`).sync} />);
    expect(screen.getByTestId("tool-sync-pct").dataset.kind).toBe(word);
    expect(screen.getByTestId("tool-sync-pct-value").textContent).toBe(word);
    expect(screen.queryByTestId("tool-sync-pct-chunks")).toBeNull();
    expect(screen.getByTestId("tool-sync-pct-value").className.includes("red")).toBe(word === "failed");
  });
  it("null → nothing", () => {
    const { container } = render(<SyncPctHeader sync={null} />);
    expect(container.innerHTML).toBe("");
  });
});

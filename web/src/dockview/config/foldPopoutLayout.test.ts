import { describe, expect, it } from "vitest";

import { foldPopoutsIntoGrid } from "./foldPopoutLayout";

describe("foldPopoutsIntoGrid", () => {
  it("returns a snapshot with no popout groups unchanged", () => {
    const layout = { grid: { root: { type: "leaf", data: { id: "center" } } } };
    expect(foldPopoutsIntoGrid(layout)).toBe(layout);
    const empty = { ...layout, popoutGroups: [] as unknown[] };
    expect(foldPopoutsIntoGrid(empty)).toBe(empty);
    const missing = { ...layout, popoutGroups: null };
    expect(foldPopoutsIntoGrid(missing)).toBe(missing);
  });

  it("folds a single popout group into the main grid and drops the popout list", () => {
    const layout = {
      grid: {
        root: {
          type: "branch",
          data: [{ type: "leaf", data: { id: "editors" }, size: 1 }],
          size: 1,
        },
      },
      popoutGroups: [{ data: { id: "browser-group" }, position: { x: 1 } }],
    };
    const next = foldPopoutsIntoGrid(layout);
    expect(next).not.toBe(layout);
    expect(next.popoutGroups).toBeUndefined();
    expect(next.grid.root.data).toEqual([
      { type: "leaf", data: { id: "editors" }, size: 1 },
      { type: "leaf", data: { id: "browser-group" }, size: 1 },
    ]);
    expect(layout.popoutGroups).toHaveLength(1);
  });

  it("wraps a lone grid leaf when a popout group comes back", () => {
    const next = foldPopoutsIntoGrid({
      grid: { root: { type: "leaf", data: { id: "only" }, size: 4 } },
      popoutGroups: [{ data: { id: "popped" } }],
    });
    expect(next.grid.root).toEqual({
      type: "branch",
      size: 4,
      data: [
        { type: "leaf", data: { id: "only" }, size: 4 },
        { type: "leaf", data: { id: "popped" }, size: 1 },
      ],
    });
  });

  it("drops popout groups it cannot place without rewriting the main grid", () => {
    const next = foldPopoutsIntoGrid({
      grid: { root: { type: "mystery", data: { id: "keep" } } },
      popoutGroups: [{ data: { id: "popped" } }],
    });
    expect(next.popoutGroups).toBeUndefined();
    expect(next.grid.root).toEqual({ type: "mystery", data: { id: "keep" } });
  });

  it("strips a popout list that is not an array and leaves the grid alone", () => {
    const next = foldPopoutsIntoGrid({
      grid: { root: { type: "leaf", data: { id: "keep" } } },
      popoutGroups: { data: { id: "nope" } },
    });
    expect(next.popoutGroups).toBeUndefined();
    expect(next.grid.root).toEqual({ type: "leaf", data: { id: "keep" } });
  });
});

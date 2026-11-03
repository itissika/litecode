import { describe, expect, it } from "vitest";

import { applyListSelection } from "./listSelection";

const rows = ["a", "b", "c", "d"];

describe("applyListSelection", () => {
  it("replaces the selection and moves the anchor on a plain click", () => {
    const result = applyListSelection(new Set(["a", "b"]), "c", "a");
    expect([...result.selected]).toEqual(["c"]);
    expect(result.anchor).toBe("c");
  });

  it("toggles additive selections and moves the anchor", () => {
    const added = applyListSelection(new Set(["a"]), "c", "a", {
      additive: true,
    });
    expect([...added.selected]).toEqual(["a", "c"]);
    expect(added.anchor).toBe("c");

    const removed = applyListSelection(added.selected, "a", added.anchor, {
      additive: true,
    });
    expect([...removed.selected]).toEqual(["c"]);
    expect(removed.anchor).toBe("a");
  });

  it("selects the inclusive visible range and keeps the anchor", () => {
    const result = applyListSelection(new Set(["a"]), "c", "a", {
      range: true,
      visible: rows,
    });
    expect([...result.selected]).toEqual(["a", "b", "c"]);
    expect(result.anchor).toBe("a");
  });

  it("selects backward ranges without reordering the result", () => {
    const result = applyListSelection(new Set(["d"]), "b", "d", {
      range: true,
      visible: rows,
    });
    expect([...result.selected]).toEqual(["b", "c", "d"]);
    expect(result.anchor).toBe("d");
  });

  it("falls back to a plain click when the range cannot resolve", () => {
    const result = applyListSelection(new Set(["a"]), "c", "missing", {
      range: true,
      visible: rows,
    });
    expect([...result.selected]).toEqual(["c"]);
    expect(result.anchor).toBe("c");
  });

  it("does not mutate the current set", () => {
    const current = new Set(["a"]);
    applyListSelection(current, "b", "a", { additive: true });
    expect([...current]).toEqual(["a"]);
  });
});

import { describe, expect, it } from "vitest";

import { nearestScrollDelta } from "./scrollVisible";

const parent = { top: 100, bottom: 300, height: 200 };

describe("nearestScrollDelta", () => {
  it("does not move a child already inside the parent", () => {
    expect(
      nearestScrollDelta(parent, { top: 120, bottom: 160, height: 40 }),
    ).toBe(0);
  });

  it("pulls a child that starts above the parent down to the top edge", () => {
    expect(
      nearestScrollDelta(parent, { top: 40, bottom: 80, height: 40 }),
    ).toBe(-60);
  });

  it("pulls a child that ends below the parent up to the bottom edge", () => {
    expect(
      nearestScrollDelta(parent, { top: 280, bottom: 340, height: 60 }),
    ).toBe(40);
  });

  it("aligns a child taller than the parent to the top", () => {
    expect(
      nearestScrollDelta(parent, { top: 140, bottom: 420, height: 280 }),
    ).toBe(40);
  });
});

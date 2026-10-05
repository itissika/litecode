import { describe, expect, it } from "vitest";

import {
  nearestUserMark,
  placeUserRailTicks,
  tickAppearance,
  USER_DOT_CENTER_Y,
} from "./transcriptUserRailMarks";

describe("tickAppearance", () => {
  it("is widest and most opaque at the midline", () => {
    const mid = tickAppearance(0, 200);
    const edge = tickAppearance(200, 200);
    expect(mid.width).toBeCloseTo(16);
    expect(edge.width).toBeCloseTo(6);
    expect(mid.opacity).toBeCloseTo(1);
    expect(edge.opacity).toBeCloseTo(0.18);
  });

  it("drops width and opacity faster than a linear ramp", () => {
    const mid = tickAppearance(0, 200);
    const halfway = tickAppearance(100, 200);
    const edge = tickAppearance(200, 200);
    expect(halfway.width).toBeLessThan((mid.width + edge.width) / 2);
    expect(halfway.opacity).toBeLessThan((mid.opacity + edge.opacity) / 2);
    expect(halfway.width).toBeGreaterThan(edge.width);
    expect(halfway.opacity).toBeGreaterThan(edge.opacity);
  });
});

describe("nearestUserMark", () => {
  it("picks the dot closest to the viewport focus", () => {
    const marks = [
      { seq: 1, contentY: 40 },
      { seq: 4, contentY: 400 },
      { seq: 9, contentY: 900 },
    ];
    expect(nearestUserMark(marks, 420)?.seq).toBe(4);
    expect(nearestUserMark([], 0)).toBeNull();
  });
});

describe("placeUserRailTicks", () => {
  const at = (ticks: { seq: number; railY: number }[], seq: number) =>
    ticks.find((tick) => tick.seq === seq)?.railY ?? NaN;

  it("pins the focused user message to the middle and keeps a fixed pitch", () => {
    const marks = [
      { seq: 1, contentY: 200 },
      { seq: 2, contentY: 400 },
      { seq: 3, contentY: 1200 },
    ];
    const placed = placeUserRailTicks(marks, 400, 200);
    expect(at(placed, 2)).toBeCloseTo(100);
    expect(at(placed, 2) - at(placed, 1)).toBeCloseTo(50);
    expect(at(placed, 3) - at(placed, 2)).toBeCloseTo(50);
  });

  it("keeps the interpolated focus at mid-band while a gap scrolls", () => {
    const marks = [
      { seq: 1, contentY: 200 },
      { seq: 2, contentY: 400 },
      { seq: 3, contentY: 1200 },
    ];
    const placed = placeUserRailTicks(marks, 600, 200);
    const fraction = (600 - 400) / (1200 - 400);
    const focus =
      at(placed, 2) + fraction * (at(placed, 3) - at(placed, 2));
    expect(focus).toBeCloseTo(100);
    expect(at(placed, 3) - at(placed, 2)).toBeCloseTo(50);
  });

  it("pins the last user message at mid-band once the focus passes it", () => {
    const marks = [
      { seq: 1, contentY: 200 },
      { seq: 2, contentY: 400 },
      { seq: 3, contentY: 1200 },
    ];
    const placed = placeUserRailTicks(marks, 4000, 200);
    expect(at(placed, 3)).toBeCloseTo(100);
    expect(at(placed, 2)).toBeCloseTo(50);
    expect(at(placed, 3)).toBeGreaterThan(at(placed, 2));
  });

  it("pins the first user message at mid-band while the focus is above it", () => {
    const marks = [
      { seq: 1, contentY: 200 },
      { seq: 2, contentY: 400 },
    ];
    const placed = placeUserRailTicks(marks, 0, 200);
    expect(at(placed, 1)).toBeCloseTo(100);
    expect(at(placed, 2)).toBeCloseTo(150);
  });
});

describe("USER_DOT_CENTER_Y", () => {
  it("matches the user-bubble gutter", () => {
    expect(USER_DOT_CENTER_Y).toBe(26);
  });
});

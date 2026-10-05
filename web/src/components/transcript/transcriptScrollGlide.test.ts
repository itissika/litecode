import { describe, expect, it } from "vitest";

import {
  centerDelta,
  easeInOutCubic,
  glideDuration,
  glideScrollTop,
  rebaseGlideFrom,
} from "./transcriptScrollGlide";

describe("easeInOutCubic", () => {
  it("holds the ends and is slowest away from the middle", () => {
    expect(easeInOutCubic(0)).toBe(0);
    expect(easeInOutCubic(1)).toBe(1);
    expect(easeInOutCubic(0.5)).toBeCloseTo(0.5);
    expect(easeInOutCubic(0.25)).toBeLessThan(0.25);
    expect(easeInOutCubic(0.75)).toBeGreaterThan(0.75);
  });
});

describe("glideScrollTop", () => {
  it("starts at from, ends at to, and lags a linear ramp early", () => {
    expect(glideScrollTop({ from: 100, to: 500, elapsed: 0, duration: 400 })).toBe(
      100,
    );
    expect(
      glideScrollTop({ from: 100, to: 500, elapsed: 400, duration: 400 }),
    ).toBe(500);
    const early = glideScrollTop({
      from: 0,
      to: 400,
      elapsed: 100,
      duration: 400,
    });
    expect(early).toBeLessThan(100);
    expect(early).toBeGreaterThan(0);
  });
});

describe("glideDuration", () => {
  it("grows with distance and stays inside the floor and cap", () => {
    expect(glideDuration(0)).toBe(340);
    expect(glideDuration(3000)).toBe(720);
    expect(glideDuration(800)).toBeGreaterThan(glideDuration(100));
  });
});

describe("rebaseGlideFrom", () => {
  it("keeps the current offset when the target moves", () => {
    const eased = easeInOutCubic(0.25);
    const current = 10 + (40 - 10) * eased;
    const from = rebaseGlideFrom(current, 80, eased);
    expect(from + (80 - from) * eased).toBeCloseTo(current);
  });
});

describe("centerDelta", () => {
  it("is zero when the element is already on the scroller midline", () => {
    expect(
      centerDelta({
        scrollerTop: 100,
        scrollerHeight: 400,
        elementTop: 250,
        elementHeight: 100,
      }),
    ).toBe(0);
  });

  it("moves only by the gap between the two midlines", () => {
    expect(
      centerDelta({
        scrollerTop: 0,
        scrollerHeight: 200,
        elementTop: 180,
        elementHeight: 40,
      }),
    ).toBe(100);
  });
});

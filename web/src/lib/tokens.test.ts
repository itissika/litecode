import { describe, expect, it } from "vitest";

import { formatToken } from "./tokens";

describe("formatToken", () => {
  it("keeps small counts exact", () => {
    expect(formatToken(0)).toBe("0");
    expect(formatToken(999)).toBe("999");
  });

  it("abbreviates thousands", () => {
    expect(formatToken(1_500)).toBe("1.5K");
    expect(formatToken(12_300)).toBe("12K");
    expect(formatToken(123_000)).toBe("123K");
  });

  it("abbreviates millions", () => {
    expect(formatToken(1_500_000)).toBe("1.5M");
    expect(formatToken(12_000_000)).toBe("12M");
    expect(formatToken(999_000_000)).toBe("999M");
  });

  it("rolls billions over instead of showing 1000M+", () => {
    expect(formatToken(1_000_000_000)).toBe("1.0B");
    expect(formatToken(1_500_000_000)).toBe("1.5B");
    expect(formatToken(12_000_000_000)).toBe("12B");
  });
});

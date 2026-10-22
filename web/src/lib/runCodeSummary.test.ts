import { describe, expect, it } from "vitest";

import { formatRunCodeData } from "./runCodeSummary";

describe("formatRunCodeData", () => {
  it("renders per-tool totals plus failed", () => {
    expect(
      formatRunCodeData({
        succeeded: 5,
        failed: 1,
        by_tool: {
          grep: { ok: 3, fail: 0 },
          read: { ok: 2, fail: 0 },
        },
        ok: true,
      }),
    ).toBe("grep ×3 · read ×2 · failed ×1");
  });

  it("counts failed calls in the tool total", () => {
    expect(
      formatRunCodeData({
        failed: 1,
        by_tool: { write: { ok: 0, fail: 1 } },
        ok: false,
      }),
    ).toBe("write ×1 · failed ×1");
  });

  it("renders a program that never started", () => {
    expect(formatRunCodeData({ ok: false, failure_kind: "syntax" })).toBe(
      "failed to start · syntax",
    );
  });

  it("adds unsettled without counting it as failed", () => {
    expect(
      formatRunCodeData({
        failed: 0,
        by_tool: { read: { ok: 2, fail: 0 } },
        unsettled: 1,
        ok: true,
      }),
    ).toBe("read ×2 · 1 unsettled");
  });

  it("ignores malformed or absent data", () => {
    expect(formatRunCodeData(null)).toBeNull();
    expect(formatRunCodeData("nope")).toBeNull();
    expect(formatRunCodeData({ by_tool: "nope" })).toBeNull();
    expect(formatRunCodeData({ ok: true })).toBeNull();
  });
});

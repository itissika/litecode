import { describe, expect, it } from "vitest";

import { formatFileTime, formatFileTimeFull } from "./fileTime";

// Local dates keep the assertions timezone-independent.
const now = new Date(2026, 9, 29, 15, 0).getTime();

describe("formatFileTime", () => {
  it("shows the clock time for files touched today", () => {
    const mtime = new Date(2026, 9, 29, 9, 5).getTime();
    expect(formatFileTime(mtime, now, "en-US")).toMatch(/0?9:05/);
  });

  it("shows month and day for files from this year", () => {
    const mtime = new Date(2026, 2, 1, 12, 0).getTime();
    expect(formatFileTime(mtime, now, "en-US")).toBe("Mar 1");
  });

  it("adds the year for older files", () => {
    const mtime = new Date(2024, 11, 24, 12, 0).getTime();
    expect(formatFileTime(mtime, now, "en-US")).toBe("12/24/2024");
  });

  it("renders a dash when the stamp is missing or invalid", () => {
    expect(formatFileTime(0, now, "en-US")).toBe("—");
    expect(formatFileTime(Number.NaN, now, "en-US")).toBe("—");
  });

  it("keeps the full stamp for the tooltip", () => {
    const mtime = new Date(2026, 9, 29, 9, 5).getTime();
    expect(formatFileTimeFull(mtime, "en-US")).toContain("2026");
    expect(formatFileTimeFull(mtime, "en-US")).toContain("09:05");
  });
});

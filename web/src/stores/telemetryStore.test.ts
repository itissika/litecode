import { describe, expect, it } from "vitest";

import {
  appendLogLineForTest,
  formatMemoryLabel,
  formatMemoryTitle,
  formatRssMb,
  memoryFromStats,
} from "./telemetryStore";

describe("formatRssMb", () => {
  it("formats kilobytes as compact megabytes", () => {
    expect(formatRssMb(43_264)).toBe("42.3M");
    expect(formatRssMb(1024)).toBe("1.0M");
    expect(formatRssMb(102_400)).toBe("100M");
  });
});

describe("memory breakdown labels", () => {
  it("shows total and core when optional engines are off", () => {
    const label = formatMemoryLabel(
      memoryFromStats({
        rss_kb: 12_288,
        core_rss_kb: 12_288,
        embed_rss_kb: 0,
        lsp_rss_kb: 0,
        ts_ms: 1,
      }),
    );
    expect(label).toBe("12.0M total · core 12.0M");
  });

  it("includes embed and lsp when non-zero", () => {
    const label = formatMemoryLabel(
      memoryFromStats({
        rss_kb: 577_536,
        core_rss_kb: 12_288,
        embed_rss_kb: 520_192,
        lsp_rss_kb: 45_056,
        ts_ms: 1,
      }),
    );
    expect(label).toContain("564M total");
    expect(label).toContain("embed 508M");
    expect(label).toContain("lsp 44.0M");
  });

  it("title lists all buckets", () => {
    const title = formatMemoryTitle(
      memoryFromStats({
        rss_kb: 12_288,
        core_rss_kb: 12_288,
        embed_rss_kb: 0,
        lsp_rss_kb: 0,
        ts_ms: 1,
      }),
    );
    expect(title).toContain("Core (serve)");
    expect(title).toContain("Embed (code_search worker)");
  });
});

describe("log ring buffer", () => {
  it("caps at 500 lines", () => {
    const lines = Array.from({ length: 520 }, (_, i) => ({
      ts_ms: i,
      level: "INFO",
      target: "t",
      message: `line ${i}`,
    }));
    const capped = appendLogLineForTest([], lines);
    expect(capped).toHaveLength(500);
    expect(capped[0]?.message).toBe("line 20");
    expect(capped[499]?.message).toBe("line 519");
  });
});

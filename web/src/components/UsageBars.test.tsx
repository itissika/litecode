import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import type { SessionUsageSeries, UsageKind, UsageMark, UsageSample } from "../api/types";
import { bucketSamples, UsageBars } from "./UsageBars";

function sample(
  seq: number,
  prompt: number,
  hit: number,
  completion = 100,
  kind: UsageKind = "request",
): UsageSample {
  return {
    seq,
    turn_id: `t-${seq}`,
    at_ms: 1_700_000_000_000 + seq,
    prompt,
    completion,
    hit,
    miss: prompt - hit,
    kind,
  };
}

function series(
  samples: UsageSample[],
  compactions: UsageMark[] = [],
  truncated = false,
): SessionUsageSeries {
  return {
    session_id: "s-1",
    samples,
    compactions,
    truncated,
    totals: {
      prompt_tokens: samples.reduce((sum, row) => sum + row.prompt, 0),
      completion_tokens: samples.reduce((sum, row) => sum + row.completion, 0),
      cache_hit_tokens: samples.reduce((sum, row) => sum + row.hit, 0),
      cache_miss_tokens: samples.reduce((sum, row) => sum + row.miss, 0),
    },
    requests: samples.length,
  };
}

afterEach(cleanup);

describe("bucketSamples", () => {
  it("keeps one bar per request when they fit", () => {
    const bars = bucketSamples(
      [sample(1, 1_000, 900), sample(2, 1_200, 1_000), sample(3, 1_400, 1_300)],
      [],
      72,
    );
    expect(bars).toHaveLength(3);
    expect(bars.map((bar) => bar.count)).toEqual([1, 1, 1]);
    expect(bars.map((bar) => bar.prompt)).toEqual([1_000, 1_200, 1_400]);
    expect(bars[1].hitShare).toBeCloseTo(1_000 / 1_200);
  });

  it("uses the water level, never a sum, when a bucket covers several requests", () => {
    const samples = Array.from({ length: 200 }, (_, index) => sample(index + 1, 1_000, 500));
    const bars = bucketSamples(samples, [], 72);
    expect(bars.length).toBeLessThanOrEqual(72);
    for (const bar of bars) {
      expect(bar.prompt).toBe(1_000);
    }
  });

  it("weights the hit split by tokens, not by row count", () => {
    const bars = bucketSamples([sample(1, 1_000, 1_000), sample(2, 3_000, 0)], [], 1);
    expect(bars).toHaveLength(1);
    expect(bars[0].hit).toBe(1_000);
    expect(bars[0].miss).toBe(3_000);
    expect(bars[0].hitShare).toBeCloseTo(0.25);
  });

  it("never lets a bucket straddle a compaction", () => {
    const samples = Array.from({ length: 10 }, (_, index) => sample(index + 1, 1_000, 500));
    const bars = bucketSamples(samples, [{ seq: 5, at_ms: 1 }], 2);
    const straddles = bars.some((bar) => samples[bar.from].seq <= 5 && samples[bar.to].seq > 5);
    expect(straddles).toBe(false);
    expect(bars.some((bar) => bar.compactedAfter)).toBe(true);
  });

  it("gives a summarizer bill its own bar so it never sets a bucket's water level", () => {
    const bars = bucketSamples(
      [
        sample(1, 1_000, 900),
        sample(2, 1_050, 900),
        sample(3, 120_000, 100_000, 1_800, "compaction"),
        sample(4, 4_000, 3_000),
        sample(5, 4_200, 3_100),
      ],
      [{ seq: 3, at_ms: 1 }],
      1,
    );
    const compaction = bars.filter((bar) => bar.compaction);
    expect(compaction).toHaveLength(1);
    expect(compaction[0].prompt).toBe(120_000);
    expect(compaction[0].count).toBe(1);
    // The turn requests around it still bucket among themselves.
    for (const bar of bars) {
      if (!bar.compaction) expect(bar.prompt).toBeLessThan(120_000);
    }
  });

  it("returns nothing for an empty session", () => {
    expect(bucketSamples([], [], 72)).toEqual([]);
  });
});

describe("UsageBars", () => {
  it("renders the empty state instead of an empty chart", () => {
    render(<UsageBars series={series([])} failed={false} />);
    expect(screen.getByText("No request usage yet")).toBeTruthy();
  });

  it("degrades to a quiet line when the read fails", () => {
    render(<UsageBars series={null} failed />);
    expect(screen.getByText("Usage temporarily unavailable")).toBeTruthy();
  });

  it("keeps the block's height while the first read is in flight", () => {
    const { container } = render(<UsageBars series={null} failed={false} />);
    expect(container.firstElementChild).toBeTruthy();
    expect(container.textContent).toBe("");
  });

  it("draws one hoverable bar per request and explains it on hover", () => {
    const { container } = render(
      <UsageBars series={series([sample(11, 2_000, 1_500), sample(12, 2_400, 1_800)], [])} failed={false} />,
    );
    expect(screen.getByRole("img", { name: /Per-request context tokens/ })).toBeTruthy();
    const hitAreas = container.querySelectorAll('rect[fill="transparent"]');
    expect(hitAreas).toHaveLength(2);
    fireEvent.mouseEnter(hitAreas[0]);
    expect(screen.getByText("prompt")).toBeTruthy();
    expect(screen.getByText("#11")).toBeTruthy();
  });

  it("names the summarizer bar as compaction in the tooltip", () => {
    const compaction = sample(3, 120_000, 100_000, 1_800, "compaction");
    const { container } = render(
      <UsageBars series={series([sample(1, 1_000, 900), compaction], [{ seq: 3, at_ms: 1 }])} failed={false} />,
    );
    const hitAreas = container.querySelectorAll('rect[fill="transparent"]');
    expect(hitAreas).toHaveLength(2);
    fireEvent.mouseEnter(hitAreas[1]);
    expect(screen.getByText("compaction")).toBeTruthy();
    expect(screen.getByText("#3")).toBeTruthy();
  });
});

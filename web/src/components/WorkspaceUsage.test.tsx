import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import type {
  WorkspaceUsageDay,
  WorkspaceUsageHour,
  WorkspaceUsageReport,
} from "../api/types";
import { bucketDays, bucketHours, WorkspaceUsageView } from "./WorkspaceUsage";

function day(
  date: string,
  prompt: number,
  hit: number,
  completion = 100,
  requests = 1,
): WorkspaceUsageDay {
  return {
    day: date,
    requests,
    prompt,
    completion,
    hit,
    miss: prompt - hit,
  };
}

function hour(
  value: number,
  prompt: number,
  hit: number,
  completion = 100,
  requests = 1,
): WorkspaceUsageHour {
  return {
    hour: value,
    requests,
    prompt,
    completion,
    hit,
    miss: prompt - hit,
  };
}

function report(
  days: WorkspaceUsageDay[],
  overrides: Partial<WorkspaceUsageReport> = {},
): WorkspaceUsageReport {
  const totals = days.reduce(
    (sum, row) => ({
      prompt_tokens: sum.prompt_tokens + row.prompt,
      completion_tokens: sum.completion_tokens + row.completion,
      cache_hit_tokens: sum.cache_hit_tokens + row.hit,
      cache_miss_tokens: sum.cache_miss_tokens + row.miss,
    }),
    { prompt_tokens: 0, completion_tokens: 0, cache_hit_tokens: 0, cache_miss_tokens: 0 },
  );
  return {
    days,
    hours: [],
    today: TODAY,
    totals,
    requests: days.reduce((sum, row) => sum + row.requests, 0),
    session_count: 1,
    ...overrides,
  };
}

const TODAY = "2026-10-22"; // Thursday; its week starts Monday 2026-10-19.

afterEach(cleanup);

describe("bucketHours", () => {
  it("keeps the whole day so the axis cannot lie about which hour a bar is", () => {
    const buckets = bucketHours([hour(9, 1_000, 900), hour(15, 200, 100)]);
    expect(buckets).toHaveLength(24);
    expect(buckets[10].prompt).toBe(0);
    expect(buckets[10].requests).toBe(0);
    expect(buckets[0].label).toBe("00");
    expect(buckets[23].label).toBe("23");
  });

  it("places a bill on its own hour", () => {
    const buckets = bucketHours([hour(13, 1_000, 900, 40, 3)]);
    expect(buckets[13]).toMatchObject({
      label: "13",
      title: "13:00",
      prompt: 1_000,
      hit: 900,
      miss: 100,
      completion: 40,
      requests: 3,
    });
  });
});

describe("bucketDays", () => {
  it("draws Monday to Sunday, leaving a quiet day as a gap in place", () => {
    const days = [
      day("2026-10-18", 500, 400), // Sunday of the previous week
      day("2026-10-19", 100, 90), // Monday
      day("2026-10-22", 200, 150),
      day("2026-10-26", 900, 800), // next week
    ];
    const buckets = bucketDays(days, "week", TODAY);
    expect(buckets.map((bucket) => bucket.key)).toEqual([
      "2026-10-19",
      "2026-10-20",
      "2026-10-21",
      "2026-10-22",
      "2026-10-23",
      "2026-10-24",
      "2026-10-25",
    ]);
    expect(buckets.map((bucket) => bucket.label)).toEqual([
      "一",
      "二",
      "三",
      "四",
      "五",
      "六",
      "日",
    ]);
    expect(buckets[0].prompt).toBe(100);
    expect(buckets[1].prompt).toBe(0);
    expect(buckets[3].prompt).toBe(200);
  });

  it("draws the 1st to the end of the month", () => {
    const buckets = bucketDays(
      [day("2026-09-30", 100, 90), day("2026-10-01", 200, 150)],
      "month",
      TODAY,
    );
    expect(buckets).toHaveLength(31);
    expect(buckets[0]).toMatchObject({ key: "2026-10-01", label: "1", prompt: 200 });
    expect(buckets[1].prompt).toBe(0);
    expect(buckets[30].key).toBe("2026-10-31");
  });

  it("runs 全部 from the first recorded day to today, gaps included", () => {
    const buckets = bucketDays(
      [day("2026-09-20", 100, 50), day("2026-10-22", 200, 100)],
      "all",
      TODAY,
    );
    // 11 (Sep 20–30) + 22 (Oct 1–22) days.
    expect(buckets).toHaveLength(33);
    expect(buckets[0].key).toBe("2026-09-20");
    expect(buckets[1].prompt).toBe(0);
    expect(buckets[buckets.length - 1].key).toBe(TODAY);
  });

  it("folds a long history down to the readable limit", () => {
    const buckets = bucketDays(
      [day("2026-08-01", 100, 50), day("2026-10-22", 200, 100)],
      "all",
      TODAY,
    );
    expect(buckets).toHaveLength(42);
    expect(buckets[0].key).toBe("2026-08-01..2026-08-02");
  });

  it("merges neighbours by summing when the frame is too long to draw", () => {
    const days = [
      day("2026-10-01", 100, 50),
      day("2026-10-02", 200, 100),
      day("2026-10-03", 300, 150),
      day("2026-10-04", 400, 200),
    ];
    const buckets = bucketDays(days, "all", "2026-10-04", 2);
    expect(buckets).toHaveLength(2);
    expect(buckets.map((bucket) => bucket.prompt)).toEqual([300, 700]);
    expect(buckets[1].days).toBe(2);
    expect(buckets[1].title).toBe("2026-10-03 … 2026-10-04");
  });
});

describe("WorkspaceUsageView", () => {
  const afternoon = report([day("2026-10-20", 3_000_000, 2_000_000, 100_000, 4)], {
    hours: [hour(9, 1_000_000, 900_000, 250_000), hour(15, 1_000_000, 950_000, 250_000)],
    session_count: 15,
  });

  it("opens on today's 24 hours, aggregating only today's bills", () => {
    render(<WorkspaceUsageView report={afternoon} />);
    expect(screen.getByText("工作区用量")).toBeTruthy();
    expect(screen.getByText("Prompt")).toBeTruthy();
    expect(screen.getByText("2.0M")).toBeTruthy();
    expect(screen.getByText("92.5%")).toBeTruthy();
    expect(screen.getByText("500K")).toBeTruthy();
    // Billed = prompt + completion, on the same authoritative rows.
    expect(screen.getByText("2.5M")).toBeTruthy();
    expect(screen.getByText("00")).toBeTruthy();
    expect(screen.getByText("23")).toBeTruthy();
  });

  it("re-scopes both the axis and the aggregates when the period changes", () => {
    render(<WorkspaceUsageView report={afternoon} />);
    fireEvent.click(screen.getByText("周"));
    expect(screen.getByText("三")).toBeTruthy(); // 2026-10-21 has no bill but exists
    expect(screen.queryByText("23")).toBeNull();
    expect(screen.getByText("3.0M")).toBeTruthy();

    fireEvent.click(screen.getByText("月"));
    expect(screen.getByText("29")).toBeTruthy();
    expect(screen.queryByText("三")).toBeNull();
    expect(screen.getByText("3.0M")).toBeTruthy();
    expect(screen.getByText("100K")).toBeTruthy();
  });

  it("answers on hover, with the same bubble the ctx ring uses", () => {
    const { container } = render(<WorkspaceUsageView report={afternoon} />);
    const slot = container.querySelector('[data-slot="h9"]');
    expect(slot).toBeTruthy();
    fireEvent.mouseEnter(slot!);
    expect(screen.getByText("09:00")).toBeTruthy();
    expect(screen.getByText("prompt")).toBeTruthy();
    expect(screen.getByText("1.0M")).toBeTruthy();
    fireEvent.mouseLeave(slot!);
    expect(screen.queryByText("09:00")).toBeNull();
  });

  it("shows the request count for the window and the workspace session count", () => {
    render(<WorkspaceUsageView report={afternoon} />);
    expect(screen.getByText(/请求 · 共 15 会话/)).toBeTruthy();
  });

  it("explains the chart with a legend under it", () => {
    render(<WorkspaceUsageView report={afternoon} />);
    expect(screen.getByText("hit")).toBeTruthy();
    expect(screen.getByText("miss")).toBeTruthy();
    expect(screen.getByText("completion")).toBeTruthy();
    expect(screen.getByRole("img", { name: /工作区 token 用量/ })).toBeTruthy();
  });

  it("keeps the empty frame and says the window has nothing", () => {
    render(<WorkspaceUsageView report={report([day("2026-08-01", 100, 50)])} />);
    // Today is empty, but the 24 slots are still the axis.
    expect(screen.getByText("该时段暂无记录")).toBeTruthy();
    expect(screen.getByText("00")).toBeTruthy();
  });

  it("no longer ranks sessions or shows compaction", () => {
    render(<WorkspaceUsageView report={afternoon} />);
    expect(screen.queryByText(/会话 · 按 prompt/)).toBeNull();
    expect(screen.queryByText(/压缩/)).toBeNull();
  });
});

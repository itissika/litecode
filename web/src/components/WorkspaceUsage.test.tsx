import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type {
  WorkspaceUsageDay,
  WorkspaceUsageHour,
  WorkspaceUsageProvider,
  WorkspaceUsageReport,
} from "../api/types";
import { useConnectionStore } from "../stores/connectionStore";
import {
  bucketDays,
  bucketHours,
  providerBuckets,
  WorkspaceUsage,
  WorkspaceUsageView,
} from "./WorkspaceUsage";

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
    providers: [],
    session_count: 1,
    ...overrides,
  };
}

function provider(
  providerId: string | null,
  days: WorkspaceUsageDay[],
  hours: WorkspaceUsageHour[] = [],
): WorkspaceUsageProvider {
  return { provider_id: providerId, days, hours };
}

const TODAY = "2026-10-22"; // Thursday; its week starts Monday 2026-10-19.

afterEach(cleanup);

function providerCards(): HTMLElement[] {
  return [...document.querySelectorAll<HTMLElement>(".usage-provider-card")];
}

function providerHeader(card: HTMLElement): HTMLElement {
  const header = card.querySelector<HTMLElement>('[role="button"]');
  if (!header) throw new Error("provider card has no FoldCard header");
  return header;
}

function expandProviderCards() {
  for (const card of providerCards()) {
    const header = providerHeader(card);
    if (header.getAttribute("aria-expanded") === "false") {
      fireEvent.click(header);
    }
  }
}

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
      "Mon",
      "Tue",
      "Wed",
      "Thu",
      "Fri",
      "Sat",
      "Sun",
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

  const withProviders = report(
    [
      day("2026-09-01", 10, 5, 1, 1),
      day("2026-10-20", 500, 250, 200, 2),
      day("2026-10-21", 50, 25, 10, 1),
      day("2026-10-22", 300, 150, 120, 2),
    ],
    {
      hours: [hour(9, 100, 50, 40, 1), hour(15, 200, 100, 80, 1)],
      providers: [
        provider(
          "openai",
          [
            day("2026-10-20", 500, 250, 200, 2),
            day("2026-10-22", 100, 50, 40, 1),
          ],
          [hour(9, 100, 50, 40, 1)],
        ),
        provider(
          "anthropic",
          [day("2026-10-22", 200, 100, 80, 1)],
          [hour(15, 200, 100, 80, 1)],
        ),
        provider(null, [day("2026-10-21", 50, 25, 10, 1)]),
        provider("old", [day("2026-09-01", 10, 5, 1, 1)]),
      ],
    },
  );

  it("opens on today's 24 hours, aggregating only today's bills", () => {
    render(<WorkspaceUsageView report={afternoon} />);
    expect(screen.getByText("Workspace usage")).toBeTruthy();
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
    fireEvent.click(screen.getByText("Week"));
    expect(screen.getByText("Wed")).toBeTruthy(); // 2026-10-21 has no bill but exists
    expect(screen.queryByText("23")).toBeNull();
    expect(screen.getByText("3.0M")).toBeTruthy();

    fireEvent.click(screen.getByText("Month"));
    expect(screen.getByText("29")).toBeTruthy();
    expect(screen.queryByText("Wed")).toBeNull();
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
    expect(screen.getByText(/15 requests · 15 sessions|requests · 15 sessions/)).toBeTruthy();
  });

  it("explains the chart with a legend under it", () => {
    render(<WorkspaceUsageView report={afternoon} />);
    expect(screen.getByText("hit")).toBeTruthy();
    expect(screen.getByText("miss")).toBeTruthy();
    expect(screen.getByText("completion")).toBeTruthy();
    expect(screen.getByRole("img", { name: /Workspace token usage/ })).toBeTruthy();
  });

  it("keeps the empty frame and says the window has nothing", () => {
    render(<WorkspaceUsageView report={report([day("2026-08-01", 100, 50)])} />);
    // Today is empty, but the 24 slots are still the axis.
    expect(screen.getByText("No records for this period")).toBeTruthy();
    expect(screen.getByText("00")).toBeTruthy();
  });

  it("no longer ranks sessions or shows compaction", () => {
    render(<WorkspaceUsageView report={afternoon} />);
    expect(screen.queryByText(/会话 · 按 prompt/)).toBeNull();
    expect(screen.queryByText(/压缩/)).toBeNull();
  });

  it("draws one chart per provider with data in the selected window", () => {
    render(<WorkspaceUsageView report={withProviders} />);
    expect(screen.getByText("By provider")).toBeTruthy();
    expect(screen.getByTestId("provider-usage-openai")).toBeTruthy();
    expect(screen.getByTestId("provider-usage-anthropic")).toBeTruthy();
    expect(screen.queryByTestId("provider-usage-unknown")).toBeNull();
    expect(screen.queryByTestId("provider-usage-old")).toBeNull();
    expandProviderCards();
    expect(
      screen.getByRole("img", { name: /openai token usage/i }),
    ).toBeTruthy();
  });

  it("collapses each provider chart into a FoldCard summary with its billed total", () => {
    render(<WorkspaceUsageView report={withProviders} />);
    const card = screen.getByTestId("provider-usage-anthropic");
    expect(card.querySelector(".foldcard")).toBeTruthy();
    expect(card.querySelector("details")).toBeNull();
    const header = providerHeader(card);
    expect(header.getAttribute("aria-expanded")).toBe("false");
    expect(header.textContent).toContain("anthropic");
    expect(header.textContent).toContain("280");
    expect(card.querySelector('[role="img"]')).toBeNull();

    fireEvent.click(header);
    expect(header.getAttribute("aria-expanded")).toBe("true");
    expect(card.querySelector('[role="img"]')).toBeTruthy();
  });

  it("sorts provider cards by the current window's billed usage", () => {
    render(<WorkspaceUsageView report={withProviders} />);
    const order = () => providerCards().map((card) => card.getAttribute("data-testid"));
    // Day: anthropic 280, openai 140.
    expect(order()).toEqual(["provider-usage-anthropic", "provider-usage-openai"]);

    fireEvent.click(screen.getByText("Week"));
    // Week: openai 840, anthropic 280, unknown 60.
    expect(order()).toEqual([
      "provider-usage-openai",
      "provider-usage-anthropic",
      "provider-usage-unknown",
    ]);
  });

  it("keeps every chart's accessible name unique", () => {
    render(<WorkspaceUsageView report={withProviders} />);
    expandProviderCards();
    const labels = screen
      .getAllByRole("img")
      .map((chart) => chart.getAttribute("aria-label") ?? "");
    expect(labels.length).toBeGreaterThanOrEqual(3);
    expect(new Set(labels).size).toBe(labels.length);
    expect(labels.some((label) => /Workspace token usage/.test(label))).toBe(true);
    expect(labels.some((label) => /openai token usage/i.test(label))).toBe(true);
  });

  it("re-scopes every provider chart with the shared period switch", () => {
    render(<WorkspaceUsageView report={withProviders} />);
    expandProviderCards();
    expect(
      screen.getByTestId("provider-usage-openai").querySelectorAll("[data-slot]"),
    ).toHaveLength(24);

    fireEvent.click(screen.getByText("Week"));
    expandProviderCards();
    expect(
      screen.getByTestId("provider-usage-openai").querySelectorAll("[data-slot]"),
    ).toHaveLength(7);
    expect(
      screen.getByTestId("provider-usage-anthropic").querySelectorAll("[data-slot]"),
    ).toHaveLength(7);
    expect(screen.queryByTestId("provider-usage-old")).toBeNull();

    fireEvent.click(screen.getByText("Month"));
    expandProviderCards();
    expect(
      screen.getByTestId("provider-usage-openai").querySelectorAll("[data-slot]"),
    ).toHaveLength(31);
    expect(
      screen.getByTestId("provider-usage-unknown").querySelectorAll("[data-slot]"),
    ).toHaveLength(31);

    fireEvent.click(screen.getByText("All"));
    expect(screen.getByTestId("provider-usage-old")).toBeTruthy();
  });

  it("shows each provider's current-window billed total and request count", () => {
    render(<WorkspaceUsageView report={withProviders} />);
    fireEvent.click(screen.getByText("Week"));
    const openai = screen.getByTestId("provider-usage-openai");
    expect(openai.textContent).toContain("840");
    expect(openai.textContent).toContain("3 requests");
  });

  it("labels the unknown bucket explicitly", () => {
    render(<WorkspaceUsageView report={withProviders} />);
    fireEvent.click(screen.getByText("Week"));
    expect(screen.getByText("Unknown (legacy compaction)")).toBeTruthy();
    expect(screen.getByTestId("provider-usage-unknown")).toBeTruthy();
  });

  it("hides the provider section when no provider paid in the window", () => {
    const pastOnly = report([day("2026-09-01", 100, 50)], {
      providers: [provider("old", [day("2026-09-01", 100, 50)])],
    });
    render(<WorkspaceUsageView report={pastOnly} />);
    expect(screen.queryByTestId("provider-usage-section")).toBeNull();
  });

  it("gives every provider the same period frame", () => {
    const rows = provider(
      "openai",
      [day("2026-10-20", 500, 250, 200, 2), day("2026-10-22", 100, 50, 40, 1)],
      [hour(9, 100, 50, 40, 1)],
    );
    expect(providerBuckets(rows, "day", TODAY)).toHaveLength(24);
    expect(providerBuckets(rows, "week", TODAY)).toHaveLength(7);
    expect(providerBuckets(rows, "month", TODAY)).toHaveLength(31);
    expect(providerBuckets(rows, "all", TODAY)).toHaveLength(3);
  });
});

describe("WorkspaceUsage popover body", () => {
  function stubRpc(sendRpc: ReturnType<typeof vi.fn>): void {
    useConnectionStore.setState({ state: "connected", sendRpc } as never);
  }

  afterEach(() => {
    useConnectionStore.setState({ sendRpc: undefined } as never);
  });

  it("keeps the block's height, and nothing else, while the first read is in flight", () => {
    stubRpc(vi.fn(() => new Promise(() => undefined)));

    const { container } = render(<WorkspaceUsage />);

    expect(container.textContent).toBe("");
    expect(container.firstElementChild).toBeTruthy();
  });

  it("says the read failed, and says something else when there is simply nothing yet", async () => {
    stubRpc(vi.fn(async () => Promise.reject(new Error("no socket"))));

    const failed = render(<WorkspaceUsage />);
    const unavailable = await screen.findByText("Usage temporarily unavailable");
    expect(unavailable).toBeTruthy();
    failed.unmount();

    stubRpc(vi.fn(async () => report([])));
    render(<WorkspaceUsage />);

    const empty = await screen.findByText("No usage records yet");
    expect(empty).toBeTruthy();
    // Two different states must never read as the same sentence.
    expect(empty.textContent).not.toBe(unavailable.textContent);
  });

  it("renders the panel once the report arrives", async () => {
    stubRpc(vi.fn(async () => report([day(TODAY, 1_000, 500)])));

    render(<WorkspaceUsage />);

    expect(await screen.findByText("Workspace usage")).toBeTruthy();
    expect(screen.getByText("Prompt")).toBeTruthy();
  });
});


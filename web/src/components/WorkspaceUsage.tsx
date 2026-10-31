import { useMemo, useState, type ReactNode } from "react";

import type {
  WorkspaceUsageDay,
  WorkspaceUsageHour,
  WorkspaceUsageProvider,
  WorkspaceUsageReport,
} from "../api/types";
import { useWorkspaceUsage } from "../api/usage";
import { useSettingsStore } from "../stores/settingsStore";
import { formatToken } from "../lib/tokens";
import { ChartTooltip, TooltipRow, tooltipLeft } from "./ChartTooltip";
import { FoldCard } from "./FoldCard";
import { ringColorForHitRate } from "./ContextUsageRing";
import i18n from "../i18n";

/** Popover content width: 240px panel shell minus 2×12px padding. */
const CHART_W = 216;
const BAR_H = 34;
/** Completion strip, on its own scale (typically ~100× smaller than prompt). */
const LINE_H = 5;
const GAP_H = 2;
/** Axis strip: bucket labels are what make the period switch legible. */
const AXIS_H = 11;
const CHART_H = BAR_H + GAP_H + LINE_H + AXIS_H;
/** Bars here are sums, so merging days is legitimate; 42 stays readable. */
const MAX_BUCKETS = 42;
const HOURS = 24;
const HIT_FILL = "var(--_dk-cat-blue)";
const MISS_FILL = "var(--_dk-amber-500)";

/**
 * The window the chart covers, and with it the axis resolution: 日 is a
 * within-day question (hours), 周 and 月 are within-month questions (days),
 * 全部 is history (days, merged when there are too many).
 */
export type Period = "day" | "week" | "month" | "all";

export type UsageBucket = {
  key: string;
  /** Axis label. */
  label: string;
  /** Hover head: the full range this bar stands for. */
  title: string;
  prompt: number;
  hit: number;
  miss: number;
  completion: number;
  requests: number;
  /** Natural days folded in; 0 for an hour bar. */
  days: number;
};

const LABEL_KEY: Record<Period, string> = {
  day: "usage.period.day",
  week: "usage.period.week",
  month: "usage.period.month",
  all: "usage.period.all",
};

function periodLabel(period: Period): string {
  return i18n.t(LABEL_KEY[period]);
}

/** What a column means, in the axis's own words. */
const AXIS_TITLE_KEY: Record<Period, string> = {
  day: "usage.axis.day",
  week: "usage.axis.week",
  month: "usage.axis.month",
  all: "usage.axis.all",
};

function axisTitle(period: Period): string {
  return i18n.t(AXIS_TITLE_KEY[period]);
}

/** Local `YYYY-MM-DD`. */
function dayKey(date: Date): string {
  const month = String(date.getMonth() + 1).padStart(2, "0");
  const dateOfMonth = String(date.getDate()).padStart(2, "0");
  return `${date.getFullYear()}-${month}-${dateOfMonth}`;
}

/** Local midnight of a `YYYY-MM-DD` day. */
function parseDay(day: string): Date {
  const date = new Date(`${day}T00:00:00`);
  return Number.isNaN(date.getTime()) ? new Date() : date;
}

/** Monday of the week that contains `day`. */
function mondayOf(day: string): Date {
  const date = parseDay(day);
  const monday = new Date(date);
  monday.setDate(date.getDate() - ((date.getDay() + 6) % 7));
  return monday;
}

type Slot = { key: string; label: string; title: string };

/** Weekday labels are their own positions: 周一 is a place on the axis. */
const WEEKDAY_KEYS = [
  "usage.weekday.mon",
  "usage.weekday.tue",
  "usage.weekday.wed",
  "usage.weekday.thu",
  "usage.weekday.fri",
  "usage.weekday.sat",
  "usage.weekday.sun",
] as const;

function weekdayLabel(index: number): string {
  return i18n.t(WEEKDAY_KEYS[index]!);
}

/**
 * Every slot the period covers, with or without a bill.
 *
 * The axis is the period itself — 24 hours, Mon..Sun, the 1st to the end of the
 * month, one column per day of history — so a quiet stretch shows as a gap in
 * place. Only slots are generated here, never numbers: an empty slot is zero
 * because no bill was written, not because something was estimated.
 */
function frameSlots(
  period: Exclude<Period, "day">,
  today: string,
  days: WorkspaceUsageDay[],
): Slot[] {
  if (period === "week") {
    const monday = mondayOf(today);
    return WEEKDAY_KEYS.map((_, index) => {
      const date = new Date(monday);
      date.setDate(monday.getDate() + index);
      const key = dayKey(date);
      return { key, label: weekdayLabel(index), title: key };
    });
  }
  if (period === "month") {
    const anchor = parseDay(today);
    // Day 0 of the next month is the last day of this one.
    const count = new Date(anchor.getFullYear(), anchor.getMonth() + 1, 0).getDate();
    return Array.from({ length: count }, (_, index) => {
      const key = dayKey(new Date(anchor.getFullYear(), anchor.getMonth(), index + 1));
      return { key, label: String(index + 1), title: key };
    });
  }
  // 全部: a column per day since the first recorded bill, up to today.
  const start = parseDay(days.length > 0 ? days[0].day : today);
  const end = parseDay(today);
  if (start > end) return [{ key: today, label: today.slice(5), title: today }];
  const slots: Slot[] = [];
  for (
    let cursor = start;
    cursor <= end && slots.length <= 4000;
    cursor = new Date(cursor.getFullYear(), cursor.getMonth(), cursor.getDate() + 1)
  ) {
    const key = dayKey(cursor);
    slots.push({ key, label: key.slice(5), title: key });
  }
  return slots;
}

/**
 * Fold authoritative days into the period's frame.
 *
 * Summing is correct here: the question is "how much did the workspace read in
 * this window", not "how full was one request". The per-request chart uses a
 * water level instead, for exactly the opposite reason.
 */
export function bucketDays(
  days: WorkspaceUsageDay[],
  period: Exclude<Period, "day">,
  today: string,
  maxBuckets: number = MAX_BUCKETS,
): UsageBucket[] {
  const byDay = new Map(days.map((row) => [row.day, row]));
  const buckets: UsageBucket[] = frameSlots(period, today, days).map((slot) => {
    const row = byDay.get(slot.key);
    return {
      key: slot.key,
      label: slot.label,
      title: slot.title,
      prompt: row?.prompt ?? 0,
      hit: row?.hit ?? 0,
      miss: row?.miss ?? 0,
      completion: row?.completion ?? 0,
      requests: row?.requests ?? 0,
      days: row ? 1 : 0,
    };
  });
  if (buckets.length <= maxBuckets) return buckets;

  // Too many to draw: merge neighbours, summing. Never average — an average
  // would understate a heavy day.
  const size = Math.ceil(buckets.length / maxBuckets);
  const coarse: UsageBucket[] = [];
  for (let from = 0; from < buckets.length; from += size) {
    const slice = buckets.slice(from, from + size);
    const folded: UsageBucket = {
      key: `${slice[0].key}..${slice[slice.length - 1].key}`,
      label: slice[0].label,
      title: `${slice[0].title} … ${slice[slice.length - 1].title}`,
      prompt: 0,
      hit: 0,
      miss: 0,
      completion: 0,
      requests: 0,
      days: 0,
    };
    for (const bucket of slice) {
      folded.prompt += bucket.prompt;
      folded.hit += bucket.hit;
      folded.miss += bucket.miss;
      folded.completion += bucket.completion;
      folded.requests += bucket.requests;
      folded.days += bucket.days;
    }
    coarse.push(folded);
  }
  return coarse;
}

/**
 * Today, one bar per hour, zero-filled.
 *
 * Unlike historical days, hours are not dropped when empty: they are the axis
 * itself here, and closing a gap would place every later label at the wrong
 * hour.
 */
export function bucketHours(hours: WorkspaceUsageHour[]): UsageBucket[] {
  const byHour = new Map(hours.map((row) => [row.hour, row]));
  return Array.from({ length: HOURS }, (_, hour) => {
    const row = byHour.get(hour);
    const label = String(hour).padStart(2, "0");
    return {
      key: `h${hour}`,
      label,
      title: `${label}:00`,
      prompt: row?.prompt ?? 0,
      hit: row?.hit ?? 0,
      miss: row?.miss ?? 0,
      completion: row?.completion ?? 0,
      requests: row?.requests ?? 0,
      days: 0,
    };
  });
}

/** Local `YYYY-MM-DD`, the fallback anchor when the server sends none. */
function localDay(): string {
  const now = new Date();
  const month = String(now.getMonth() + 1).padStart(2, "0");
  const date = String(now.getDate()).padStart(2, "0");
  return `${now.getFullYear()}-${month}-${date}`;
}

/** dot · name · amount, the same row rhythm as the ctx ring popover. */
function AggregateRow({
  color,
  label,
  value,
}: {
  color: string;
  label: string;
  value: string;
}) {
  return (
    <div className="flex items-center justify-between gap-2">
      <span className="flex items-center gap-1.5 text-(--_dk-text-secondary)">
        <span
          aria-hidden
          className="h-1 w-1 rounded-full"
          style={{ backgroundColor: color }}
        />
        {label}
      </span>
      <span className="font-mono tabular-nums text-(--_dk-text-secondary)">
        {value}
      </span>
    </div>
  );
}

function LegendItem({ children }: { children: ReactNode }) {
  return <span className="flex items-center gap-1">{children}</span>;
}

function Note({ children }: { children: ReactNode }) {
  return (
    <div className="px-3 py-3 text-[11px] text-(--_dk-text-disabled)">
      {children}
    </div>
  );
}

/** Window totals from the same buckets the chart draws. */
type WindowTotals = {
  prompt: number;
  hit: number;
  completion: number;
  /** Prompt + completion: the same "Billed" total the aggregates show. */
  billed: number;
  requests: number;
};

function windowTotals(buckets: UsageBucket[]): WindowTotals {
  return buckets.reduce(
    (acc, bucket) => ({
      prompt: acc.prompt + bucket.prompt,
      hit: acc.hit + bucket.hit,
      completion: acc.completion + bucket.completion,
      billed: acc.billed + bucket.prompt + bucket.completion,
      requests: acc.requests + bucket.requests,
    }),
    { prompt: 0, hit: 0, completion: 0, billed: 0, requests: 0 },
  );
}

/**
 * One stacked hit/miss chart with a completion hairline. The same component
 * draws the workspace total and every provider, so a period change cannot make
 * one chart disagree with another.
 */
function UsageChart({
  buckets,
  period,
  ariaLabel,
}: {
  buckets: UsageBucket[];
  period: Period;
  ariaLabel: string;
}) {
  const [hover, setHover] = useState<number | null>(null);
  const totals = windowTotals(buckets);
  const empty = totals.requests === 0 && totals.prompt === 0;
  const maxPrompt = Math.max(1, ...buckets.map((bucket) => bucket.prompt));
  const maxCompletion = Math.max(1, ...buckets.map((bucket) => bucket.completion));
  // Roomy columns: a handful of buckets should read as labelled columns, not
  // hairlines — the period switch is legible only if each bar can carry its key.
  const pitch = Math.max(
    1,
    Math.min(Math.floor(CHART_W / Math.max(1, buckets.length)), 48),
  );
  const gapW = pitch >= 3 ? 1 : 0;
  const barW = Math.max(1, pitch - gapW);
  const offsetX = Math.round((CHART_W - pitch * buckets.length) / 2);
  const hovered = hover !== null ? (buckets[hover] ?? null) : null;
  const bubbleLeft = tooltipLeft(
    offsetX + (hover ?? 0) * pitch + pitch / 2,
    CHART_W,
  );
  // Ticks on the frame's own ladder: every 6 hours, every weekday, every 7th
  // day of the month. 全部 has no ladder, so it thins by label width instead.
  const axisIndices = (() => {
    const last = buckets.length - 1;
    if (last <= 0) return [0];
    const ladder =
      period === "week" ? 1 : period === "day" ? 6 : period === "month" ? 7 : 0;
    const step =
      ladder > 0
        ? ladder
        : Math.max(
            1,
            Math.ceil((Math.max(...buckets.map((bucket) => bucket.label.length)) * 4.5 + 10) / pitch),
          );
    const indices: number[] = [];
    for (let index = 0; index < buckets.length; index += step) indices.push(index);
    // 月 的最后一格会贴上前一个标签，其余留个右边界。
    if (period !== "month" && indices[indices.length - 1] !== last) indices.push(last);
    return indices;
  })();

  const completionPoints = buckets
    .map((bucket, index) => {
      const x = offsetX + index * pitch + pitch / 2;
      const h = Math.max(1, (bucket.completion / maxCompletion) * (LINE_H - 1));
      return x.toFixed(1) + "," + (BAR_H + GAP_H + LINE_H - h).toFixed(1);
    })
    .join(" ");

  return (
    <div className="relative" style={{ width: CHART_W }}>
      <svg
        width={CHART_W}
        height={CHART_H}
        viewBox={"0 0 " + CHART_W + " " + CHART_H}
        role="img"
        aria-label={ariaLabel}
      >
        {buckets.map((bucket, index) => {
          const h =
            bucket.prompt > 0
              ? Math.max(1, Math.round((bucket.prompt / maxPrompt) * BAR_H))
              : 0;
          const hitShare = bucket.prompt > 0 ? bucket.hit / bucket.prompt : 0;
          const hitH = Math.round(h * hitShare);
          const missH = Math.max(0, h - hitH);
          const x = offsetX + index * pitch + (pitch - barW) / 2;
          return (
            <g key={bucket.key}>
              <rect
                x={x}
                y={BAR_H - hitH}
                width={barW}
                height={hitH}
                fill={HIT_FILL}
                fillOpacity={0.45}
              />
              <rect
                x={x}
                y={BAR_H - h}
                width={barW}
                height={missH}
                fill={MISS_FILL}
              />
              <rect
                x={offsetX + index * pitch}
                y={0}
                width={pitch}
                height={CHART_H}
                fill="transparent"
                data-slot={bucket.key}
                onMouseEnter={() => setHover(index)}
                onMouseLeave={() => setHover(null)}
              />
            </g>
          );
        })}
        <line
          x1={0}
          x2={CHART_W}
          y1={BAR_H + GAP_H - 0.5}
          y2={BAR_H + GAP_H - 0.5}
          stroke="var(--_dk-line)"
          strokeWidth={1}
        />
        <polyline
          points={completionPoints}
          fill="none"
          stroke="var(--_dk-text-muted)"
          strokeWidth={1}
        />
        {empty && (
          <text
            x={CHART_W / 2}
            y={BAR_H / 2}
            fontSize={9}
            fill="var(--_dk-text-disabled)"
            textAnchor="middle"
          >
            {i18n.t("usage.noRecordsPeriod")}
          </text>
        )}
        {axisIndices.map((index) => {
          const bucket = buckets[index];
          const last = buckets.length - 1;
          const isFirst = index === 0;
          const isLast = index === last;
          return (
            <text
              key={"axis-" + bucket.key}
              x={
                isFirst
                  ? offsetX
                  : isLast
                    ? offsetX + pitch * buckets.length
                    : offsetX + index * pitch + pitch / 2
              }
              y={CHART_H - 1}
              fontSize={8}
              fill="var(--_dk-text-disabled)"
              textAnchor={isFirst ? "start" : isLast ? "end" : "middle"}
              className="font-mono"
            >
              {bucket.label}
            </text>
          );
        })}
      </svg>
      {hovered && (
        <ChartTooltip left={bubbleLeft} bottom={CHART_H + 4}>
          <TooltipRow
            muted
            label={hovered.title}
            value={hovered.requests > 0 ? i18n.t("usage.requestCount", { count: hovered.requests }) : null}
          />
          <TooltipRow label="prompt" value={formatToken(hovered.prompt)} />
          <TooltipRow
            label="hit / miss"
            value={formatToken(hovered.hit) + " / " + formatToken(hovered.miss)}
          />
          <TooltipRow
            label="completion"
            value={formatToken(hovered.completion)}
          />
        </ChartTooltip>
      )}
    </div>
  );
}

/** The same frame and arithmetic the workspace chart uses, for one provider. */
export function providerBuckets(
  provider: WorkspaceUsageProvider,
  period: Period,
  today: string,
): UsageBucket[] {
  return period === "day"
    ? bucketHours(provider.hours)
    : bucketDays(provider.days, period, today);
}

/** The panel body: aggregates first, the chart that explains them second. */
export function WorkspaceUsageView({ report }: { report: WorkspaceUsageReport }) {
  const [period, setPeriod] = useState<Period>("day");
  const today = report.today || localDay();
  const buckets = useMemo(
    () =>
      period === "day"
        ? bucketHours(report.hours)
        : bucketDays(report.days, period, today),
    [period, report.days, report.hours, today],
  );

  // The aggregates cover the selected window, so they always add up to the
  // chart directly under them.
  const totals = windowTotals(buckets);
  const hitRate = totals.prompt > 0 ? totals.hit / totals.prompt : null;
  const billed = totals.billed;

  const catalog = useSettingsStore((state) => state.llm?.providers);
  const providerNames = useMemo(
    () => new Map((catalog ?? []).map((provider) => [provider.id, provider.name] as const)),
    [catalog],
  );
  const providerName = (providerId: string | null): string => {
    if (!providerId) return i18n.t("usage.providerUnknown");
    return providerNames.get(providerId) ?? providerId;
  };

  // One provider row per provider that paid a bill in this window. The same
  // period state drives every chart, so switching the grain cannot leave one
  // provider on a different axis. Cards are ordered by the same billed total
  // the summary shows, not by the chart's hit/miss breakdown.
  const providerViews = useMemo(
    () =>
      (report.providers ?? [])
        .map((provider) => {
          const providerDays = providerBuckets(provider, period, today);
          return { provider, buckets: providerDays, totals: windowTotals(providerDays) };
        })
        .filter((view) => view.totals.requests > 0)
        .sort((a, b) => {
          const byBilled = b.totals.billed - a.totals.billed;
          if (byBilled !== 0) return byBilled;
          const byRequests = b.totals.requests - a.totals.requests;
          if (byRequests !== 0) return byRequests;
          const aId = a.provider.provider_id;
          const bId = b.provider.provider_id;
          if (aId === null) return bId === null ? 0 : 1;
          if (bId === null) return -1;
          return aId < bId ? -1 : aId > bId ? 1 : 0;
        }),
    [report.providers, period, today],
  );

  return (
    <div className="flex flex-col gap-2 px-3 py-3 text-[11px]">
      <div className="flex items-baseline justify-between gap-2">
        <span className="text-(--_dk-text-secondary)">{i18n.t("usage.workspaceTitle")}</span>
        <span className="text-[10px] text-(--_dk-text-disabled)">
          {i18n.t("usage.requestsSessions", { requests: totals.requests.toLocaleString(), sessions: report.session_count })}
        </span>
      </div>

      {/* Aggregates: what was read, how much of it was cached, what it cost. */}
      <div className="flex flex-col gap-0.5 border-t border-(--_dk-line) pt-2.5">
        <AggregateRow
          color={HIT_FILL}
          label="Prompt"
          value={formatToken(totals.prompt)}
        />
        <AggregateRow
          color={hitRate !== null ? ringColorForHitRate(hitRate) : "var(--_dk-text-disabled)"}
          label="Cache hit"
          value={hitRate !== null ? (hitRate * 100).toFixed(1) + "%" : "--"}
        />
        <AggregateRow
          color="var(--_dk-text-muted)"
          label="Completion"
          value={formatToken(totals.completion)}
        />
        <AggregateRow
          color="var(--_dk-text-disabled)"
          label="Billed"
          value={formatToken(billed)}
        />
      </div>

      {/* Breakdown: the same rows, folded into buckets and drawn. */}
      <div className="flex flex-col gap-1.5 border-t border-(--_dk-line) pt-2.5">
        <div className="flex items-center justify-end gap-0.5">
          {(Object.keys(LABEL_KEY) as Period[]).map((key) => (
            <button
              key={key}
              type="button"
              onClick={() => setPeriod(key)}
              aria-pressed={period === key}
              className={[
                "rounded px-1 py-0.5 text-[10px] transition-colors",
                period === key
                  ? "bg-(--_dk-ix-bg-hover) text-(--_dk-text-primary)"
                  : "text-(--_dk-text-disabled) hover:text-(--_dk-text-secondary)",
              ].join(" ")}
            >
              {periodLabel(key)}
            </button>
          ))}
        </div>

        <UsageChart
          key={period}
          buckets={buckets}
          period={period}
          ariaLabel={i18n.t("usage.chartAria", { axis: axisTitle(period) })}
        />

        <div className="flex items-center gap-3 text-[10px] text-(--_dk-text-disabled)">
          <LegendItem>
            <span
              aria-hidden
              className="h-1.5 w-1.5 rounded-[1px]"
              style={{ backgroundColor: HIT_FILL, opacity: 0.45 }}
            />
            {"hit"}
          </LegendItem>
          <LegendItem>
            <span
              aria-hidden
              className="h-1.5 w-1.5 rounded-[1px]"
              style={{ backgroundColor: MISS_FILL }}
            />
            {"miss"}
          </LegendItem>
          <LegendItem>
            <span
              aria-hidden
              className="h-px w-2.5"
              style={{ backgroundColor: "var(--_dk-text-muted)" }}
            />
            {"completion"}
          </LegendItem>
        </div>
      </div>

      {providerViews.length > 0 && (
        <div
          data-testid="provider-usage-section"
          className="flex flex-col gap-2 border-t border-(--_dk-line) pt-2.5"
        >
          <span className="text-(--_dk-text-secondary)">{i18n.t("usage.byProvider")}</span>
          {providerViews.map(({ provider, buckets: providerBucket, totals: providerTotals }) => {
            // A real provider id may itself be "unknown"; prefix named ids so
            // the React key cannot collide with the legacy unknown bucket.
            const key =
              provider.provider_id === null ? "unknown" : "provider:" + provider.provider_id;
            const testId = provider.provider_id ?? "unknown";
            const name = providerName(provider.provider_id);
            return (
              <div
                key={key}
                data-testid={"provider-usage-" + testId}
                className="usage-provider-card"
              >
                <FoldCard
                  defaultOpen={false}
                  readyOnMount
                  label={
                    <span className="flex w-full min-w-0 items-baseline justify-between gap-2">
                      <span className="min-w-0 truncate text-(--_dk-text-secondary)">
                        {name}
                      </span>{" "}
                      <span className="shrink-0 font-mono tabular-nums text-[10px] text-(--_dk-text-disabled)">
                        {formatToken(providerTotals.billed)} ·{" "}
                        {i18n.t("usage.requestCount", { count: providerTotals.requests })}
                      </span>
                    </span>
                  }
                  className="usage-foldcard"
                  headerClassName="text-[11px] text-(--_dk-text-muted)"
                  frameColor="var(--_dk-overlay)"
                  edgeBlur={false}
                >
                  <UsageChart
                    key={period}
                    buckets={providerBucket}
                    period={period}
                    ariaLabel={i18n.t("usage.providerChartAria", {
                      provider: name,
                      axis: axisTitle(period),
                    })}
                  />
                </FoldCard>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}

/** Popover body: reads the workspace once per open, then renders it. */
export function WorkspaceUsage() {
  const { report, failed } = useWorkspaceUsage();
  if (failed) return <Note>{i18n.t("usage.unavailable")}</Note>;
  if (!report) return <div style={{ height: 64 }} aria-hidden />;
  if (report.days.length === 0 && report.hours.length === 0) {
    return <Note>{i18n.t("usage.noRecords")}</Note>;
  }
  return <WorkspaceUsageView report={report} />;
}

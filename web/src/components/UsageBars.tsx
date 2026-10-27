import { useMemo, useState } from "react";

import type { SessionUsageSeries, UsageMark, UsageSample } from "../api/types";
import { formatToken } from "../lib/tokens";
import { ChartTooltip, TooltipRow, tooltipLeft } from "./ChartTooltip";
import i18n from "../i18n";

/** Popover content width: 240px panel shell minus 2×12px padding. */
const CHART_W = 216;
/** Stacked bars: prompt = hit + miss. */
const BAR_H = 36;
/** Completion strip under the bars, on its own scale (typically ~100× smaller). */
const LINE_H = 6;
const GAP_H = 2;
const CHART_H = BAR_H + GAP_H + LINE_H;
/** Readable ceiling: 72 bars at a 3px pitch. */
const MAX_BARS = 72;

export type UsageBar = {
  /** First sample index this bar covers (inclusive). */
  from: number;
  /** Last sample index this bar covers (inclusive). */
  to: number;
  count: number;
  /** Water level: the last request's prompt tokens in this bar. */
  prompt: number;
  hit: number;
  miss: number;
  /** Token-weighted cached share (0..1). */
  hitShare: number;
  /** Mean completion tokens per request in this bar. */
  completion: number;
  /** This bar `is` the summarizer request, not a turn request. */
  compaction: boolean;
  /** A compaction lands right after this bar. */
  compactedAfter: boolean;
};

/**
 * Group samples into at most `maxBars` bars, fine to coarse.
 *
 * Bar height is the *last* prompt in a bucket, never a sum — a sum would grow
 * with the bucket width and read as context growth. The hit/miss split is
 * token-weighted for the same reason. Buckets never straddle a compaction, so
 * its step down can never be averaged away.
 */
export function bucketSamples(
  samples: UsageSample[],
  compactions: UsageMark[],
  maxBars: number = MAX_BARS,
): UsageBar[] {
  if (samples.length === 0) return [];
  const size = Math.max(1, Math.ceil(samples.length / Math.max(1, maxBars)));

  // Index of the first sample that came after each compaction.
  const breaks = new Set<number>();
  for (const mark of compactions) {
    const index = samples.findIndex((sample) => sample.seq > mark.seq);
    const at = index === -1 ? samples.length : index;
    if (at > 0 && at < samples.length) breaks.add(at);
  }

  const bars: UsageBar[] = [];
  let from = 0;
  while (from < samples.length) {
    // A summarizer bill stands alone: it reads the whole pre-compaction context,
    // so blending it into a neighbour bucket would hand that bucket someone
    // else's water level.
    if (samples[from].kind === "compaction") {
      const row = samples[from];
      const total = row.hit + row.miss;
      bars.push({
        from,
        to: from,
        count: 1,
        prompt: row.prompt,
        hit: row.hit,
        miss: row.miss,
        hitShare: total > 0 ? row.hit / total : 0,
        completion: row.completion,
        compaction: true,
        compactedAfter: breaks.has(from + 1),
      });
      from += 1;
      continue;
    }
    let to = Math.min(from + size, samples.length);
    // Snap forward to the first compaction edge inside this bucket: either the
    // row after a checkpoint, or a summarizer bill, which must stay alone.
    for (let i = from + 1; i <= to; i++) {
      if (breaks.has(i) || (i < samples.length && samples[i].kind === "compaction")) {
        to = i;
        break;
      }
    }
    let hit = 0;
    let miss = 0;
    let completion = 0;
    for (let i = from; i < to; i++) {
      hit += samples[i].hit;
      miss += samples[i].miss;
      completion += samples[i].completion;
    }
    const total = hit + miss;
    bars.push({
      from,
      to: to - 1,
      count: to - from,
      prompt: samples[to - 1].prompt,
      hit,
      miss,
      hitShare: total > 0 ? hit / total : 0,
      completion: completion / (to - from),
      compaction: false,
      compactedAfter: breaks.has(to),
    });
    from = to;
  }
  return bars;
}

function Note({ children }: { children: string }) {
  return (
    <div
      className="flex items-center text-[11px] text-(--_dk-text-disabled)"
      style={{ height: CHART_H }}
    >
      {children}
    </div>
  );
}

/**
 * Per-request token bills for one session: stacked hit/miss bars (bar height =
 * that request's prompt) plus a completion hairline on its own scale.
 *
 * Server-owned numbers only — this component never estimates or scales a value
 * to make the picture look better. Empty, loading and failure states keep the
 * block's height so the popover does not jump.
 */
export function UsageBars({
  series,
  failed,
}: {
  /** The caller's series for this session; `null` until a read lands. */
  series: SessionUsageSeries | null;
  /** The last read failed, so `series` is not merely slow. */
  failed: boolean;
}) {
  const [hover, setHover] = useState<number | null>(null);
  const bars = useMemo(
    () => (series ? bucketSamples(series.samples, series.compactions) : []),
    [series],
  );

  if (failed) return <Note>{i18n.t("usage.unavailable")}</Note>;
  if (!series) return <div style={{ height: CHART_H }} />;
  if (bars.length === 0) return <Note>{i18n.t("usage.noRequestUsage")}</Note>;

  const maxPrompt = Math.max(1, ...bars.map((bar) => bar.prompt));
  const maxCompletion = Math.max(1, ...bars.map((bar) => bar.completion));
  // Cap the pitch so a handful of requests reads as bars, not one slab.
  const pitch = Math.max(1, Math.min(Math.floor(CHART_W / bars.length), 8));
  const gapW = pitch >= 3 ? 1 : 0;
  const barW = Math.max(1, pitch - gapW);
  const offsetX = Math.round((CHART_W - pitch * bars.length) / 2);
  const lastSeq = series.samples[series.samples.length - 1].seq;
  const tailCompacted = series.compactions.some((mark) => mark.seq >= lastSeq);
  const hovered = hover !== null ? bars[hover] : null;
  const hoverIndex = hover ?? 0;

  const completionPoints = bars
    .map((bar, index) => {
      const x = offsetX + index * pitch + pitch / 2;
      const h = Math.max(1, (bar.completion / maxCompletion) * (LINE_H - 1));
      const y = BAR_H + GAP_H + LINE_H - h;
      return `${x.toFixed(1)},${y.toFixed(1)}`;
    })
    .join(" ");

  const bubbleLeft = tooltipLeft(
    offsetX + hoverIndex * pitch + pitch / 2,
    CHART_W,
  );

  return (
    <div className="relative" style={{ width: CHART_W }}>
      <svg
        width={CHART_W}
        height={CHART_H}
        viewBox={`0 0 ${CHART_W} ${CHART_H}`}
        role="img"
        aria-label={i18n.t("usage.perRequestAria")}
      >
        {bars.map((bar, index) => {
          const h = Math.max(1, Math.round((bar.prompt / maxPrompt) * BAR_H));
          const hitH = Math.round(h * bar.hitShare);
          const missH = Math.max(0, h - hitH);
          const x = offsetX + index * pitch + (pitch - barW) / 2;
          return (
            <g key={`${bar.from}-${bar.to}`}>
              <rect
                x={x}
                y={BAR_H - hitH}
                width={barW}
                height={hitH}
                fill="var(--_dk-cat-blue)"
                fillOpacity={0.45}
              />
              <rect
                x={x}
                y={BAR_H - h}
                width={barW}
                height={missH}
                fill="var(--_dk-amber-500)"
              />
              <rect
                x={offsetX + index * pitch}
                y={0}
                width={pitch}
                height={CHART_H}
                fill="transparent"
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
        {bars.map((bar, index) =>
          bar.compactedAfter ? (
            <line
              key={`c-${bar.to}`}
              x1={offsetX + (index + 1) * pitch - 0.5}
              x2={offsetX + (index + 1) * pitch - 0.5}
              y1={0}
              y2={CHART_H}
              stroke="var(--_dk-text-disabled)"
              strokeWidth={1}
              strokeDasharray="2 2"
            />
          ) : null,
        )}
        {tailCompacted && (
          <line
            x1={offsetX + bars.length * pitch - 0.5}
            x2={offsetX + bars.length * pitch - 0.5}
            y1={0}
            y2={CHART_H}
            stroke="var(--_dk-text-disabled)"
            strokeWidth={1}
            strokeDasharray="2 2"
          />
        )}
      </svg>
      {hovered && (
        <ChartTooltip left={bubbleLeft} bottom={CHART_H + 4}>
          <TooltipRow
            muted
            label={
              <>
                #{series.samples[hovered.from].seq}
                {hovered.count > 1 ? `–${series.samples[hovered.to].seq}` : ""}
              </>
            }
            value={
              hovered.compaction ? "compaction" : hovered.count > 1 ? i18n.t("usage.requestCount", { count: hovered.count }) : null
            }
          />
          <TooltipRow label="prompt" value={formatToken(hovered.prompt)} />
          <TooltipRow
            label="hit / miss"
            value={`${formatToken(hovered.hit)} / ${formatToken(hovered.miss)}`}
          />
          <TooltipRow
            label="completion"
            value={formatToken(Math.round(hovered.completion))}
          />
        </ChartTooltip>
      )}
    </div>
  );
}

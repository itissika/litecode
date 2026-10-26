import type { CSSProperties } from "react";
import { ArrowsInSimple, CircleNotch } from "@phosphor-icons/react";
import { useTurnStore } from "../stores/turnStore";
import { useConnectionStore } from "../stores/connectionStore";
import type { ItemTokenBreakdown } from "../api/types";
import { formatToken } from "../lib/tokens";
import { FoldCard } from "./FoldCard";
import { Popover } from "./ui/Popover";
import { useSessionUsage } from "../api/usage";
import { UsageBars } from "./UsageBars";

function cacheHitRate(hit: number, miss: number): number | null {
  const total = hit + miss;
  if (total === 0) return null;
  return hit / total;
}

type RGB = [number, number, number];

function hexToRgb(hex: string): RGB {
  const m = hex.match(/^#([0-9a-f]{6})$/i);
  if (!m) return [0, 0, 0];
  const n = parseInt(m[1], 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

function lerp(a: number, b: number, t: number): number {
  return a + (b - a) * t;
}

// Cache-hit-rate → color. With healthy prefix caching hit rates sit at 80–95%+;
// 60% already signals cache utilization problems, so it is fully red. The scale
// is squeezed toward the top: yellow tier (0.8) is where the breathing glow
// kicks in, green is reserved for genuinely good rates.
//
// Colors are a fixed, theme-independent set (the brighter dark-theme hues), so
// the ring reads the same on dark and light surfaces.
const RING_STOPS: { color: string; at: number }[] = [
  { color: "#ef4444", at: 0 },
  { color: "#ef4444", at: 0.6 },
  { color: "#f97316", at: 0.7 },
  { color: "#eab308", at: 0.8 },
  { color: "#06b6d4", at: 0.9 },
  { color: "#10b981", at: 1 },
];

// Exported for Remotion compositions (c-switch ring replica) so the hit-rate →
// hue mapping stays single-sourced with the product component.
export function ringColorForHitRate(rate: number): string {
  const clamped = Math.max(0, Math.min(1, rate));
  let lo = RING_STOPS[0];
  let hi = RING_STOPS[RING_STOPS.length - 1];
  for (let i = 0; i < RING_STOPS.length - 1; i++) {
    if (clamped >= RING_STOPS[i].at && clamped <= RING_STOPS[i + 1].at) {
      lo = RING_STOPS[i];
      hi = RING_STOPS[i + 1];
      break;
    }
  }
  const span = hi.at - lo.at || 1;
  const t = (clamped - lo.at) / span;
  const a = hexToRgb(lo.color);
  const b = hexToRgb(hi.color);
  const r = Math.round(lerp(a[0], b[0], t));
  const g = Math.round(lerp(a[1], b[1], t));
  const bl = Math.round(lerp(a[2], b[2], t));
  return `rgb(${r}, ${g}, ${bl})`;
}

type OccupancySegment = {
  key: string;
  label: string;
  tokens: number;
  color: string;
};

type ToolOccupancy = { name: string; tokens: number };

// Local cl100k split, scaled so the three named buckets add up to the
// occupancy numerator. Tool rows add up to tool definitions plus the tool
// calls and outputs inside Messages.
function occupancyMix(
  used: number,
  bd: ItemTokenBreakdown | undefined,
): { tiers: OccupancySegment[]; tools: ToolOccupancy[] } | null {
  if (!bd || used <= 0) return null;
  const system = Math.max(
    0,
    (bd.system ?? 0) + (bd.prompt_overhead ?? 0),
  );
  const toolDefs = Math.max(0, bd.tool_schema ?? 0);
  const toolCall = Math.max(0, bd.tool_call ?? 0);
  const toolOutput = Math.max(0, bd.tool_output ?? 0);
  const messages = Math.max(0, (bd.conversation ?? 0) + toolCall + toolOutput);
  const classified = system + toolDefs + messages;
  if (classified <= 0) return null;
  const systemTokens = Math.round((system / classified) * used);
  const toolTokens = Math.round((toolDefs / classified) * used);
  const messageTokens = Math.max(0, used - systemTokens - toolTokens);
  const tiers: OccupancySegment[] = [
    {
      key: "system",
      label: "System",
      tokens: systemTokens,
      color: "var(--_dk-cat-red)",
    },
    {
      key: "tools",
      label: "Tool definitions",
      tokens: toolTokens,
      color: "var(--_dk-amber-500)",
    },
    {
      key: "messages",
      label: "Messages",
      tokens: messageTokens,
      color: "var(--_dk-cat-blue)",
    },
  ].filter((row) => row.tokens > 0);

  const toolClassified = toolDefs + toolCall + toolOutput;
  const toolTarget = Math.round((toolClassified / classified) * used);
  const raw = (bd.per_tool ?? [])
    .map((row) => ({
      name: row.name,
      weight: (row.schema ?? 0) + (row.call ?? 0) + (row.output ?? 0),
    }))
    .filter((row) => row.weight > 0 && row.name.length > 0);
  const weightSum = raw.reduce((sum, row) => sum + row.weight, 0);
  const tools: ToolOccupancy[] = [];
  if (weightSum > 0 && toolTarget > 0) {
    let assigned = 0;
    raw.forEach((row, index) => {
      const tokens =
        index === raw.length - 1
          ? Math.max(0, toolTarget - assigned)
          : Math.round((row.weight / weightSum) * toolTarget);
      assigned += tokens;
      if (tokens > 0) tools.push({ name: row.name, tokens });
    });
  }
  return { tiers, tools };
}

export function ContextUsageRing({
  sessionId,
  readOnly = false,
}: {
  sessionId: string;
  /** Subagent panels: the ring still reports occupancy and cache hit, but the
   *  Compaction action (a write) stays in the human-owned primary panel. */
  readOnly?: boolean;
}) {
  // Occupancy is server-owned. `contextMeter` is the single authority for used
  // tokens and manual eligibility; raw provider prompt is only cache-hit truth.
  // Legacy context_tokens_estimate is a display fallback when an older server
  // snapshot omitted the meter, never a local threshold input.
  const meter = useTurnStore(
    (s) => s.byId.get(sessionId)?.contextMeter ?? null,
  );
  const estimate = useTurnStore(
    (s) => s.byId.get(sessionId)?.contextTokensEstimate ?? 0,
  );
  const providerPrompt = useTurnStore(
    (s) => s.byId.get(sessionId)?.lastTurnPromptTokens ?? 0,
  );
  const used = meter?.used_tokens ?? (estimate > 0 ? estimate : null);
  const contextWindow = useTurnStore(
    (s) => s.byId.get(sessionId)?.contextWindow ?? 0,
  );
  const total = meter?.context_window ?? contextWindow;
  const compactEligible = useTurnStore(
    (s) => s.byId.get(sessionId)?.compactEligible ?? false,
  );
  const cacheHit = useTurnStore(
    (s) => s.byId.get(sessionId)?.lastTurnCacheHitTokens ?? 0,
  );
  const cacheMiss = useTurnStore(
    (s) => s.byId.get(sessionId)?.lastTurnCacheMissTokens ?? 0,
  );
  // Session-total accumulators (Σ per-request usage; token-weighted aggregate
  // hit rate — industry convention, e.g. LiteLLM Σcached/Σinput).
  const sessionCacheHit = useTurnStore(
    (s) => s.byId.get(sessionId)?.sessionCacheHitTokens ?? 0,
  );
  const sessionCacheMiss = useTurnStore(
    (s) => s.byId.get(sessionId)?.sessionCacheMissTokens ?? 0,
  );
  const compacting = useTurnStore(
    (s) => s.byId.get(sessionId)?.compacting ?? false,
  );
  const runState = useTurnStore(
    (s) => s.byId.get(sessionId)?.runState ?? "idle",
  );
  const compact = useTurnStore((s) => s.compact);
  const connected = useConnectionStore((s) => s.state === "connected");
  const breakdown = useTurnStore(
    (s) => s.byId.get(sessionId)?.contextTokenBreakdown,
  );
  const sessionBilledTokens = useTurnStore(
    (s) => s.byId.get(sessionId)?.sessionBilledTokens ?? null,
  );
  // The usage read is mounted with the popover content, never with the ring:
  // a closed ring must not pay for a chart nobody opened.
  const usageKey = `${runState}:${compacting ? 1 : 0}:${sessionBilledTokens ?? 0}`;

  const hasOccupancy = used !== null && used > 0;
  const hasProviderTruth = providerPrompt > 0;
  const pct = hasOccupancy && total > 0 ? Math.min(used / total, 1) : 0;
  const hitRate = hasProviderTruth ? cacheHitRate(cacheHit, cacheMiss) : null;
  const sessionHitRate = cacheHitRate(sessionCacheHit, sessionCacheMiss);

  // Ring color encodes cache hit rate when truth is present.
  // No provider usage yet → neutral gray (absent, not alarming).
  const color =
    hitRate !== null
      ? ringColorForHitRate(hitRate)
      : "var(--_dk-text-disabled)";
  const sessionColor =
    sessionHitRate !== null
      ? ringColorForHitRate(sessionHitRate)
      : "var(--_dk-text-disabled)";

  const size = 30;
  const radius = 11;
  const circumference = 2 * Math.PI * radius;
  const dashOffset = circumference * (1 - pct);

  // Glow is an alpha, not a boolean: hit rate 90%→60% maps linearly onto
  // alpha 0→1. Lower hit rate = stronger, more urgent glow; 90%+ is calm.
  const glowAlpha =
    hitRate === null
      ? 0
      : Math.max(0, Math.min(1, (0.9 - hitRate) / (0.9 - 0.6)));

  // Occupancy bar reads red / yellow / blue. Red and blue are the categorical
  // pair set; yellow is the theme-tuned amber, because --_dk-cat-yellow turns
  // mustard and looks dirty on light surfaces. Kept deliberately distinct from
  // the cache-hit green→red hue so the two metrics are never confused.
  const occupancyBarWidth = hasOccupancy && total > 0 ? `${pct * 100}%` : "0%";
  const mix = hasOccupancy ? occupancyMix(used, breakdown) : null;
  // Compaction point on the occupancy bar, as a share of the selected window.
  // compact_at is server-owned (min(80% window, prompt limit)); the bar is
  // scaled to context_window, so the ratio maps straight onto the track.
  const compactMarkPct =
    meter && total > 0 && meter.compact_at > 0
      ? Math.min(meter.compact_at / total, 1) * 100
      : null;
  const segments = mix?.tiers ?? [];
  const toolRows = mix?.tools ?? [];
  const hasMix = segments.length > 0;
  const compactDisabled =
    !connected || runState !== "idle" || compacting || !compactEligible;

  return (
    <Popover
      width={240}
      trigger={({ open, toggle }) => (
        <button
          type="button"
          onClick={toggle}
          aria-expanded={open}
          className="group relative flex h-[30px] w-[30px] shrink-0 items-center justify-center rounded-md"
          title="Context usage"
        >
          <div className="flex items-center justify-center transition-[filter,transform] duration-150 group-hover:scale-110 group-hover:brightness-110 group-active:scale-90 group-active:brightness-90">
            <svg
              width={28}
              height={28}
              viewBox={`0 0 ${size} ${size}`}
              className="-rotate-90"
            >
              <circle
                cx={size / 2}
                cy={size / 2}
                r={radius}
                fill="none"
                stroke="var(--_dk-text-disabled)"
                strokeWidth={2.5}
              />
              {pct > 0 && glowAlpha > 0 && (
                <g style={{ opacity: glowAlpha }}>
                  <circle
                    cx={size / 2}
                    cy={size / 2}
                    r={radius}
                    fill="none"
                    stroke={color}
                    strokeWidth={4}
                    strokeLinecap="round"
                    strokeDasharray={circumference}
                    strokeDashoffset={dashOffset}
                    className="dk-ring-glow"
                  />
                </g>
              )}
              {pct > 0 && (
                <circle
                  cx={size / 2}
                  cy={size / 2}
                  r={radius}
                  fill="none"
                  stroke={color}
                  strokeWidth={3}
                  strokeLinecap="round"
                  strokeDasharray={circumference}
                  strokeDashoffset={dashOffset}
                />
              )}
            </svg>
            <span className="absolute inset-0 flex items-center justify-center text-dk-3xs font-mono tabular-nums text-(--_dk-text-muted)">
              {hasOccupancy && total > 0 ? Math.round(pct * 100) : "--"}
            </span>
          </div>
        </button>
      )}
    >
      {({}) => (
        <div className="flex flex-col gap-3 px-3 py-3 text-[11px]">
          {/* Context occupancy + compact action */}
          <div className="flex items-center gap-2">
            <div className="flex min-w-0 flex-1 flex-col gap-1.5">
              <div className="flex items-baseline justify-between gap-2">
                <span className="text-(--_dk-text-muted)">Context</span>
                <span className="font-mono tabular-nums text-(--_dk-text-secondary)">
                  {used !== null ? formatToken(used) : "--"}
                  {" / "}
                  {total > 0 ? formatToken(total) : "--"}
                  {hasOccupancy && total > 0 && (
                    <span className="ml-1 text-(--_dk-text-muted)">
                      -{(pct * 100).toFixed(1)}%
                    </span>
                  )}
                </span>
              </div>
              <div className="relative h-1.5 w-full">
                <div className="h-full w-full overflow-hidden rounded-full bg-(--_dk-line)">
                  {hasMix ? (
                    <div
                      className="flex h-full"
                      style={{ width: occupancyBarWidth }}
                    >
                      {segments.map((seg) => (
                        <div
                          key={seg.key}
                          className="h-full min-w-px"
                          style={{
                            width: `${(seg.tokens / (used ?? 1)) * 100}%`,
                            backgroundColor: seg.color,
                          }}
                          title={`${seg.label} ${formatToken(seg.tokens)}`}
                        />
                      ))}
                    </div>
                  ) : (
                    <div
                      className="h-full rounded-full bg-(--_dk-text-muted)"
                      style={{ width: occupancyBarWidth }}
                    />
                  )}
                </div>
                {compactMarkPct !== null && (
                  // Decorative compaction point: a slider thumb where auto
                  // compaction fires. Position only, no numbers.
                  <div
                    aria-hidden
                    className="pointer-events-none absolute inset-y-0"
                    style={{ left: `${compactMarkPct}%` }}
                  >
                    <span className="absolute -top-[3px] left-0 h-3 w-[3px] -translate-x-1/2 rounded-[2px] bg-(--_dk-text-secondary) shadow-[0_0_0_1px_var(--_dk-overlay)]" />
                  </div>
                )}
              </div>
              {hasMix && (
                <FoldCard
                  defaultOpen={false}
                  label="Estimate"
                  className="mt-0.5"
                  headerClassName="text-[11px] text-(--_dk-text-muted)"
                  contentClassName="px-0 pb-0.5"
                  frameColor="var(--_dk-overlay)"
                  edgeBlur={false}
                >
                  <div className="flex flex-col gap-0.5">
                    {segments.map((seg) => (
                      <div
                        key={seg.key}
                        className="flex items-center justify-between gap-2"
                      >
                        <span className="flex min-w-0 items-center gap-1.5 text-(--_dk-text-secondary)">
                          <span
                            className="h-1 w-1 shrink-0 rounded-full"
                            style={{ backgroundColor: seg.color }}
                          />
                          <span className="truncate">{seg.label}</span>
                        </span>
                        <span className="font-mono tabular-nums text-(--_dk-text-secondary)">
                          {formatToken(seg.tokens)}
                        </span>
                      </div>
                    ))}
                    {toolRows.length > 0 && (
                      <>
                        <span className="mt-1 text-(--_dk-text-disabled)">
                          Per tool
                        </span>
                        {toolRows.map((row) => (
                          <div
                            key={row.name}
                            className="flex items-center justify-between gap-2"
                          >
                            <span className="min-w-0 truncate font-mono text-(--_dk-text-secondary)">
                              {row.name}
                            </span>
                            <span className="font-mono tabular-nums text-(--_dk-text-secondary)">
                              {formatToken(row.tokens)}
                            </span>
                          </div>
                        ))}
                      </>
                    )}
                    <span className="text-(--_dk-text-disabled)">
                      Estimated, not billed
                    </span>
                  </div>
                </FoldCard>
              )}
              {!hasOccupancy && (
                <span className="text-(--_dk-text-disabled)">
                  No context usage yet
                </span>
              )}
              {hasOccupancy && total === 0 && (
                <span className="text-(--_dk-text-disabled)">
                  No context window configured
                </span>
              )}
            </div>
            {!readOnly && (
              <button
                type="button"
                disabled={compactDisabled}
                onClick={() => compact(sessionId)}
                title={compacting ? "Compacting…" : "Compact context"}
                aria-label={
                  compacting ? "Compacting context" : "Compact context"
                }
                className="flex h-7 w-7 shrink-0 items-center justify-center rounded-md border border-(--_dk-line) text-(--_dk-text-secondary) transition-colors hover:bg-(--_dk-ix-bg-hover) disabled:cursor-not-allowed disabled:opacity-40"
              >
                {compacting ? (
                  <CircleNotch
                    size={14}
                    weight="bold"
                    className="animate-spin"
                    aria-hidden
                  />
                ) : (
                  <ArrowsInSimple size={14} weight="bold" aria-hidden />
                )}
              </button>
            )}
          </div>

          <UsageSection
            sessionId={sessionId}
            usageKey={usageKey}
            sessionHitRate={sessionHitRate}
            sessionColor={sessionColor}
            hitRate={hitRate}
            color={color}
          />
        </div>
      )}
    </Popover>
  );
}

function UsageSection({
  sessionId,
  usageKey,
  sessionHitRate,
  sessionColor,
  hitRate,
  color,
}: {
  sessionId: string;
  usageKey: string;
  sessionHitRate: number | null;
  sessionColor: string;
  hitRate: number | null;
  color: string;
}) {
  const usage = useSessionUsage(sessionId, usageKey);
  const billedTokens = usage.series
    ? usage.series.totals.prompt_tokens + usage.series.totals.completion_tokens
    : null;

  return (
    <div className="flex flex-col gap-2 border-t border-(--_dk-line) pt-2.5">
      <UsageBars series={usage.series} failed={usage.failed} />

      {/* Cache hit rate — session-total aggregate + last request; dot colors
          follow the same ring hue logic. */}
      <div className="flex flex-col gap-1">
        <span className="text-(--_dk-text-muted)">Cache hit</span>
        <div className="flex items-center justify-between">
          <span className="flex items-center gap-1.5 text-(--_dk-text-secondary)">
            <span
              className="h-1 w-1 rounded-full"
              style={{ backgroundColor: sessionColor }}
            />
            Total
          </span>
          <span className="font-mono tabular-nums text-(--_dk-text-secondary)">
            {sessionHitRate !== null
              ? `${(sessionHitRate * 100).toFixed(1)}%`
              : "--"}
          </span>
        </div>
        <div className="flex items-center justify-between">
          <span className="flex items-center gap-1.5 text-(--_dk-text-secondary)">
            <span
              className="h-1 w-1 rounded-full"
              style={
                {
                  backgroundColor: color,
                  ["--_dk-cache-pulse-color" as string]: color,
                  animation:
                    hitRate !== null
                      ? "dk-cache-pulse 1.8s ease-in-out infinite"
                      : "none",
                } as CSSProperties
              }
            />
            Current
          </span>
          <span className="font-mono tabular-nums text-(--_dk-text-secondary)">
            {hitRate !== null ? `${(hitRate * 100).toFixed(1)}%` : "--"}
          </span>
        </div>
      </div>

      <div className="flex items-center justify-between gap-2 text-(--_dk-text-secondary)">
        <span>Billed</span>
        <span className="font-mono tabular-nums">
          {billedTokens != null ? formatToken(billedTokens) : "-"}
        </span>
      </div>
    </div>
  );
}


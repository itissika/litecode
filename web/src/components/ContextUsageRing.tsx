import type { CSSProperties } from "react";
import { ArrowsInSimple, CircleNotch } from "@phosphor-icons/react";
import { useTurnStore } from "../stores/turnStore";
import { useConnectionStore } from "../stores/connectionStore";
import type { ItemTokenBreakdown, ToolTokenRow } from "../api/types";
import { FoldCard } from "./FoldCard";
import { Popover } from "./ui/Popover";

function formatToken(n: number): string {
  // K/M abbreviation keeps the 240px popover readable: 12.3K, 123K, 1.2M.
  // Sub-1000 stays exact (cheap, and loses nothing at small magnitudes).
  if (n < 1000) return n.toLocaleString();
  if (n < 10_000) return `${(n / 1000).toFixed(1)}K`;
  if (n < 1_000_000) return `${Math.round(n / 1000)}K`;
  if (n < 10_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  return `${Math.round(n / 1_000_000)}M`;
}

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

function occupancySegments(
  used: number,
  bd: ItemTokenBreakdown,
): OccupancySegment[] {
  const parts: OccupancySegment[] = [
    { key: "system", label: "System", tokens: Math.max(0, bd.system ?? 0), color: "var(--_dk-cat-cyan)" },
    { key: "schema", label: "Tool schemas", tokens: Math.max(0, bd.tool_schema ?? 0), color: "var(--_dk-cat-orange)" },
    { key: "call", label: "Tool calls", tokens: Math.max(0, bd.tool_call ?? 0), color: "var(--_dk-cat-purple)" },
    { key: "output", label: "Tool outputs", tokens: Math.max(0, bd.tool_output ?? 0), color: "var(--_dk-cat-blue)" },
    { key: "conv", label: "Conversation", tokens: Math.max(0, bd.conversation ?? 0), color: "var(--_dk-cat-pink)" },
  ];
  let classified = parts.reduce((s, p) => s + p.tokens, 0);
  let other = Math.max(0, used - classified);
  if (classified > used && classified > 0) {
    const scale = used / classified;
    for (const p of parts) p.tokens = Math.round(p.tokens * scale);
    classified = parts.reduce((s, p) => s + p.tokens, 0);
    other = Math.max(0, used - classified);
  }
  parts.push({
    key: "other",
    label: "Other",
    tokens: other,
    color: "var(--_dk-text-disabled)",
  });
  return parts.filter((p) => p.tokens > 0);
}

function usedToolRows(bd: ItemTokenBreakdown | undefined): ToolTokenRow[] {
  return (bd?.per_tool ?? []).filter(
    (row) => (row.schema ?? 0) + (row.call ?? 0) + (row.output ?? 0) > 0,
  );
}

export function ContextUsageRing({ sessionId }: { sessionId: string }) {
  // Live occupancy prefers provider prompt_tokens (updates each llm_completed
  // step). After compact the backend clears last-turn stats, so we fall back to
  // the working-set estimate until the next main-model call.
  const estimate = useTurnStore(
    (s) => s.byId.get(sessionId)?.contextTokensEstimate ?? 0,
  );
  const providerPrompt = useTurnStore(
    (s) => s.byId.get(sessionId)?.lastTurnPromptTokens ?? 0,
  );
  const used = providerPrompt > 0 ? providerPrompt : estimate;
  const total = useTurnStore(
    (s) => s.byId.get(sessionId)?.contextWindow ?? 0,
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
  const turnCompletion = useTurnStore(
    (s) => s.byId.get(sessionId)?.lastTurnCompletionTokens ?? 0,
  );
  const compactEligible = useTurnStore(
    (s) => s.byId.get(sessionId)?.compactEligible ?? false,
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

  const hasOccupancy = used > 0;
  const hasProviderTruth = providerPrompt > 0;
  const pct = hasOccupancy && total > 0 ? Math.min(used / total, 1) : 0;
  const hitRate = hasProviderTruth ? cacheHitRate(cacheHit, cacheMiss) : null;
  const sessionHitRate = cacheHitRate(sessionCacheHit, sessionCacheMiss);

  // Ring color encodes cache hit rate when truth is present.
  // No provider usage yet → neutral gray (absent, not alarming).
  const color = hitRate !== null ? ringColorForHitRate(hitRate) : "var(--_dk-text-disabled)";
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

  // Occupancy bar uses a neutral light/dark accent pair — deliberately distinct
  // from the cache-hit green→red hue so the two metrics are never confused.
  const occupancyBarWidth = hasOccupancy && total > 0 ? `${pct * 100}%` : "0%";
  const segments =
    hasOccupancy && breakdown
      ? occupancySegments(used, breakdown)
      : [];
  const hasMix = segments.some((s) => s.key !== "other");
  const toolRows = usedToolRows(breakdown);
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
      {({ }) => (
        <div className="flex flex-col gap-3 px-3 py-3 text-[11px]">
          {/* Context occupancy + compact action */}
          <div className="flex items-center gap-2">
            <div className="flex min-w-0 flex-1 flex-col gap-1.5">
              <div className="flex items-baseline justify-between gap-2">
                <span className="text-(--_dk-text-muted)">Context</span>
                <span className="font-mono tabular-nums text-(--_dk-text-secondary)">
                  {hasOccupancy ? formatToken(used) : "--"}
                  {" / "}
                  {total > 0 ? formatToken(total) : "--"}
                  {hasOccupancy && total > 0 && (
                    <span className="ml-1 text-(--_dk-text-muted)">
                      -{(pct * 100).toFixed(1)}%
                    </span>
                  )}
                </span>
              </div>
              <div className="h-1.5 w-full overflow-hidden rounded-full bg-(--_dk-line)">
                {hasMix ? (
                  <div className="flex h-full" style={{ width: occupancyBarWidth }}>
                    {segments.map((seg) => (
                      <div
                        key={seg.key}
                        className="h-full min-w-px"
                        style={{
                          width: `${(seg.tokens / used) * 100}%`,
                          backgroundColor: seg.color,
                        }}
                        title={`${seg.label} ${formatToken(seg.tokens)} (est.)`}
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
                      <div key={seg.key} className="flex items-center justify-between gap-2">
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
                        <span className="mt-1 text-(--_dk-text-disabled)">Per tool</span>
                        {toolRows.map((row) => {
                          const usedByTool =
                            (row.schema ?? 0) + (row.call ?? 0) + (row.output ?? 0);
                          return (
                            <div
                              key={row.name}
                              className="flex items-center justify-between gap-2"
                              title={`schema ${formatToken(row.schema ?? 0)} · call ${formatToken(row.call ?? 0)} · output ${formatToken(row.output ?? 0)}`}
                            >
                              <span className="min-w-0 truncate font-mono text-(--_dk-text-secondary)">
                                {row.name}
                              </span>
                              <span className="font-mono tabular-nums text-(--_dk-text-secondary)">
                                {formatToken(usedByTool)}
                              </span>
                            </div>
                          );
                        })}
                      </>
                    )}
                    <span className="text-(--_dk-text-disabled)">Estimated, not billed</span>
                  </div>
                </FoldCard>
              )}
              {!hasOccupancy && (
                <span className="text-(--_dk-text-disabled)">No context usage yet</span>
              )}
              {hasOccupancy && total === 0 && (
                <span className="text-(--_dk-text-disabled)">No context window configured</span>
              )}
            </div>
            <button
              type="button"
              disabled={compactDisabled}
              onClick={() => compact(sessionId)}
              title={compacting ? "Compacting…" : "Compact context"}
              aria-label={compacting ? "Compacting context" : "Compact context"}
              className="flex h-7 w-7 shrink-0 items-center justify-center rounded-md border border-(--_dk-line) text-(--_dk-text-secondary) transition-colors hover:bg-(--_dk-ix-bg-hover) disabled:cursor-not-allowed disabled:opacity-40"
            >
              {compacting ? (
                <CircleNotch size={14} weight="bold" className="animate-spin" aria-hidden />
              ) : (
                <ArrowsInSimple size={14} weight="bold" aria-hidden />
              )}
            </button>
          </div>

          {/* Cache hit rate — session-total aggregate + last request; dot colors
              follow the same ring hue logic */}
          <div className="flex flex-col gap-1 border-t border-(--_dk-line) pt-2.5">
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
                {sessionHitRate !== null ? `${(sessionHitRate * 100).toFixed(1)}%` : "--"}
              </span>
            </div>
            <div className="flex items-center justify-between">
              <span className="flex items-center gap-1.5 text-(--_dk-text-secondary)">
                <span
                  className="h-1 w-1 rounded-full"
                  style={{
                    backgroundColor: color,
                    ["--_dk-cache-pulse-color" as string]: color,
                    animation:
                      hitRate !== null
                        ? "dk-cache-pulse 1.8s ease-in-out infinite"
                        : "none",
                  } as CSSProperties}
                />
                Current
              </span>
              <span className="font-mono tabular-nums text-(--_dk-text-secondary)">
                {hitRate !== null ? `${(hitRate * 100).toFixed(1)}%` : "--"}
              </span>
            </div>
          </div>

          {hasProviderTruth && (
            <div className="flex items-center justify-between border-t border-(--_dk-line) pt-2.5 text-(--_dk-text-disabled)">
              <span>Last request</span>
              <span className="font-mono tabular-nums">
                {formatToken(providerPrompt)} prompt + {formatToken(turnCompletion)} completion
              </span>
            </div>
          )}
        </div>
      )}
    </Popover>
  );
}
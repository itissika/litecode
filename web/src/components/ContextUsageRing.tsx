import type { CSSProperties } from "react";
import { ArrowsInSimple, CircleNotch } from "@phosphor-icons/react";
import { useTurnStore } from "../stores/turnStore";
import { useConnectionStore } from "../stores/connectionStore";
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

function parseVar(name: string): RGB {
  // Resolve a CSS custom property to an [r,g,b] triple at runtime so the
  // gradient follows the active theme (dark/light).
  if (typeof window === "undefined") return [0, 0, 0];
  const raw = getComputedStyle(document.documentElement)
    .getPropertyValue(name)
    .trim();
  const m = raw.match(/^#([0-9a-f]{6})$/i);
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
const RING_STOPS: { name: string; at: number }[] = [
  { name: "--_dk-red-500", at: 0 },
  { name: "--_dk-red-500", at: 0.6 },
  { name: "--_dk-cat-orange", at: 0.7 },
  { name: "--_dk-cat-yellow", at: 0.8 },
  { name: "--_dk-cat-cyan", at: 0.9 },
  { name: "--_dk-emerald-500", at: 1 },
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
  const a = parseVar(lo.name);
  const b = parseVar(hi.name);
  const r = Math.round(lerp(a[0], b[0], t));
  const g = Math.round(lerp(a[1], b[1], t));
  const bl = Math.round(lerp(a[2], b[2], t));
  return `rgb(${r}, ${g}, ${bl})`;
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
                <div
                  className="h-full rounded-full bg-(--_dk-text-muted)"
                  style={{ width: occupancyBarWidth }}
                />
              </div>
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
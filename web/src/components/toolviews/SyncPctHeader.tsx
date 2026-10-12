import type { SyncPct } from "./syncPct";

/**
 * Compact sync% line at the top of code_search / session_search result views.
 * Renders nothing when `sync` is null (missing field → omit, never fake 100%).
 */
export function SyncPctHeader({ sync }: { sync: SyncPct | null }) {
  if (!sync) return null;
  return (
    <div
      className="flex items-baseline gap-1.5 px-1.5 font-mono text-dk-xs tabular-nums text-(--_dk-text-muted)"
      data-testid="tool-sync-pct"
    >
      <span data-testid="tool-sync-pct-value">{sync.pct}%</span>
      {sync.label ? (
        <span
          className="text-(--_dk-text-disabled)"
          data-testid="tool-sync-pct-label"
        >
          {sync.label}
        </span>
      ) : null}
    </div>
  );
}

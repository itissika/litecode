import type { SyncPct } from "./syncPct";

/**
 * Compact sync% line at the top of code_search / session_search result views
 * (WP-057 v2). Renders nothing when `sync` is null — never a fake value.
 */
export function SyncPctHeader({ sync }: { sync: SyncPct | null }) {
  if (!sync) return null;
  const value = sync.kind === "pct" ? `${sync.pct}%` : sync.kind;
  return (
    <div
      className="flex items-baseline gap-1.5 px-1.5 font-mono text-dk-xs tabular-nums text-(--_dk-text-muted)"
      data-testid="tool-sync-pct"
      data-kind={sync.kind}
    >
      <span
        data-testid="tool-sync-pct-value"
        className={sync.kind === "failed" ? "text-(--_dk-red-500)" : undefined}
      >
        {value}
      </span>
      {sync.kind === "pct" && sync.chunks ? (
        <span data-testid="tool-sync-pct-chunks">
          {sync.chunks.done}/{sync.chunks.total} chunks
        </span>
      ) : null}
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

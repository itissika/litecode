/**
 * Parse the WP-057 sync% first line from tool result text.
 *
 * Contract: leading `sync_pct: <n>%` (optional `(label)`). Missing / invalid →
 * omit (never invent 100%). Body after the line is returned as `rest`.
 */

export interface SyncPct {
  /** Integer 0–100 from the first line. */
  pct: number;
  /** Optional label inside parentheses, e.g. `syncing`. */
  label?: string;
}

export interface SyncPctSplit {
  sync: SyncPct | null;
  /** Output with the sync first line removed when present; otherwise raw. */
  rest: string;
}

const SYNC_LINE_RE =
  /^\s*sync_pct:\s*(\d{1,3})%\s*(?:\(([^)]+)\))?\s*$/;

/**
 * Split tool output into an optional sync header and the remaining body.
 * Only the first line is considered; a bad or out-of-range value leaves the
 * whole text as `rest` and `sync: null`.
 */
export function splitSyncPct(raw: string): SyncPctSplit {
  if (!raw) return { sync: null, rest: raw };
  const normalized = raw.replace(/\r\n/g, "\n").replace(/\r/g, "\n");
  const nl = normalized.indexOf("\n");
  const first = nl === -1 ? normalized : normalized.slice(0, nl);
  const match = SYNC_LINE_RE.exec(first);
  if (!match) return { sync: null, rest: raw };
  const pct = Number(match[1]);
  if (!Number.isInteger(pct) || pct < 0 || pct > 100) {
    return { sync: null, rest: raw };
  }
  const label = match[2]?.trim();
  const rest = nl === -1 ? "" : normalized.slice(nl + 1);
  return {
    sync: label ? { pct, label } : { pct },
    rest,
  };
}

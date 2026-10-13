/**
 * Parse the WP-057 **v2** sync% first line from tool result text (WP-138).
 *
 * `sync_pct: <value>[ (<label>)][ chunks <done>/<total>]`
 * - value: `<0–100>%` | `scanning` | `failed` | `unknown` (lower-case only)
 * - label: ASCII parens, no nested parens / newline, non-empty
 * - chunks: only after a `<n>%` value; non-negative integers (done ≤ total
 *   is the backend's job)
 * - single spaces between segments; nothing else on the line
 *
 * Anything else → `sync: null` and the text is returned untouched (the line
 * stays in the body; we never invent a value). No v1 compatibility path.
 */

export type SyncWord = "scanning" | "failed" | "unknown";

export type SyncPct =
  | {
      kind: "pct";
      /** Integer 0–100. */
      pct: number;
      label?: string;
      chunks?: { done: number; total: number };
    }
  | { kind: SyncWord; label?: string };

export interface SyncPctSplit {
  sync: SyncPct | null;
  /** Output with the sync first line removed when recognised; otherwise raw. */
  rest: string;
}

const SYNC_LINE_RE =
  /^sync_pct: (?:(\d{1,3})%|(scanning|failed|unknown))(?: \(([^()\n]+)\))?(?: chunks (\d+)\/(\d+))?$/;

export function parseSyncPctLine(line: string): SyncPct | null {
  const m = SYNC_LINE_RE.exec(line);
  if (!m) return null;
  const [, num, word, label, done, total] = m;
  const withLabel = label !== undefined ? { label } : {};
  if (word !== undefined) {
    if (done !== undefined) return null; // chunks only with a number
    return { kind: word as SyncWord, ...withLabel };
  }
  const pct = Number(num);
  if (!Number.isInteger(pct) || pct < 0 || pct > 100) return null;
  return {
    kind: "pct",
    pct,
    ...withLabel,
    ...(done !== undefined
      ? { chunks: { done: Number(done), total: Number(total) } }
      : {}),
  };
}

/** Split tool output into an optional sync header and the remaining body. */
export function splitSyncPct(raw: string): SyncPctSplit {
  if (!raw) return { sync: null, rest: raw };
  const normalized = raw.replace(/\r\n/g, "\n").replace(/\r/g, "\n");
  const nl = normalized.indexOf("\n");
  const first = nl === -1 ? normalized : normalized.slice(0, nl);
  const sync = parseSyncPctLine(first);
  if (!sync) return { sync: null, rest: raw };
  return { sync, rest: nl === -1 ? "" : normalized.slice(nl + 1) };
}

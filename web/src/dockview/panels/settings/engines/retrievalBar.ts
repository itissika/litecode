import type {
  RetrievalCorpusError,
  RetrievalCorpusStatus,
  RetrievalEngineDetail,
} from "../../../../api/workspace";

/**
 * WP-136: the retrieval progress bar is derived ONLY from the backend's merged
 * `retrieval.{total,code,session}` (WP-132). No estimates, no fixed widths,
 * no legacy index progress / percentage / chunk-count fields.
 */
type BarTone = "ok" | "pending" | "busy" | "failed" | "empty";

interface RetrievalBar {
  tone: BarTone;
  pulse: boolean;
  /** 0–100, or null when unknown (scanning / no total). */
  pct: number | null;
  indeterminate: boolean;
}

/** done/total over the whole index; null when total unknown or 0. */
function corpusPct(c: RetrievalCorpusStatus): number | null {
  if (c.chunks_total === null || c.chunks_total <= 0) return null;
  const pct = Math.floor((c.chunks_done * 100) / c.chunks_total);
  return Math.max(0, Math.min(100, pct));
}

function isScanning(c: RetrievalCorpusStatus): boolean {
  return c.state === "refreshing" && c.chunks_total === null;
}

export function retrievalBar(
  r: Pick<RetrievalEngineDetail, "total" | "code" | "session">,
): RetrievalBar {
  const t = r.total;
  // Red wins: either side failed (total.state merges failed first).
  if (
    t.state === "failed" ||
    r.code.state === "failed" ||
    r.session.state === "failed"
  ) {
    return { tone: "failed", pulse: false, pct: corpusPct(t) ?? 100, indeterminate: false };
  }
  if (t.state === "needs_rebuild") {
    return { tone: "empty", pulse: false, pct: 0, indeterminate: false };
  }
  if (t.state === "refreshing") {
    if (isScanning(t)) {
      return { tone: "busy", pulse: true, pct: null, indeterminate: true };
    }
    return { tone: "busy", pulse: true, pct: corpusPct(t), indeterminate: false };
  }
  // ready — "has pending work" is files_pending only (WP-142); chunks_pending
  // never drives colour, and neither feeds the percentage.
  if (t.files_pending > 0) {
    return { tone: "pending", pulse: false, pct: corpusPct(t), indeterminate: false };
  }
  return { tone: "ok", pulse: false, pct: corpusPct(t) ?? 100, indeterminate: false };
}

const REASON_TEXT: Record<RetrievalCorpusError["reason"], string> = {
  model_missing: "模型缺失",
  engine_unavailable: "引擎不可用",
  engine_stopped: "引擎已停止",
  index_failed: "索引失败",
};

function reasonText(reason: RetrievalCorpusError["reason"]): string {
  return REASON_TEXT[reason] ?? reason;
}

function pctLabel(c: RetrievalCorpusStatus): string {
  if (isScanning(c)) return "scanning";
  const p = corpusPct(c);
  return p === null ? "—" : `${p}%`;
}

function corpusLine(name: string, c: RetrievalCorpusStatus): string {
  const head = `${name} ${pctLabel(c)}`;
  if (c.chunks_total === null) return head;
  return `${head} · ${c.chunks_done}/${c.chunks_total} chunks`;
}

/** Hover: Total / Code / Session lines (+ failure line when red). */
export function retrievalTip(
  r: Pick<RetrievalEngineDetail, "total" | "code" | "session">,
): string {
  const lines = [
    `Total ${pctLabel(r.total)}`,
    corpusLine("Code", r.code),
    corpusLine("Session", r.session),
  ];
  const err = r.total.error ?? r.code.error ?? r.session.error;
  if (err && retrievalBar(r).tone === "failed") {
    lines.push(
      `Failed: ${err.engine} · ${reasonText(err.reason)}${err.message ? ` — ${err.message}` : ""}`,
    );
  }
  return lines.join("\n");
}

/** Poll faster while the backend is moving (or just after a refresh click). */
export function retrievalBusy(r: Pick<RetrievalEngineDetail, "total">): boolean {
  return r.total.state === "refreshing";
}

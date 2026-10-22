/**
 * One-line human summary for `run_code`'s persisted UI data.
 *
 * Wire shape (subset):
 * `{ succeeded, failed, duration_ms, by_tool, ok, failure_kind?, unsettled? }`
 * — never derived from the model-facing content text.
 */
export function formatRunCodeData(data: unknown): string | null {
  if (data == null || typeof data !== "object" || Array.isArray(data)) {
    return null;
  }
  const rec = data as Record<string, unknown>;
  const parts: string[] = [];

  const byTool = rec.by_tool;
  if (byTool && typeof byTool === "object" && !Array.isArray(byTool)) {
    const entries: Array<{ tool: string; count: number }> = [];
    for (const [tool, value] of Object.entries(byTool)) {
      if (!value || typeof value !== "object" || Array.isArray(value)) continue;
      const counts = value as Record<string, unknown>;
      const ok = nonNegativeInt(counts.ok) ?? 0;
      const fail = nonNegativeInt(counts.fail) ?? 0;
      const count = ok + fail;
      if (count > 0) entries.push({ tool, count });
    }
    entries.sort((a, b) =>
      a.count === b.count
        ? a.tool < b.tool
          ? -1
          : a.tool > b.tool
            ? 1
            : 0
        : b.count - a.count,
    );
    for (const entry of entries) parts.push(`${entry.tool} ×${entry.count}`);
  }

  const failed = nonNegativeInt(rec.failed) ?? 0;
  if (failed > 0) parts.push(`failed ×${failed}`);

  const unsettled = nonNegativeInt(rec.unsettled) ?? 0;
  if (unsettled > 0) parts.push(`${unsettled} unsettled`);

  if (parts.length > 0) return parts.join(" · ");

  // A program that never started has no counts, only a failure kind.
  if (rec.ok === false) {
    const kind = typeof rec.failure_kind === "string" ? rec.failure_kind : null;
    return kind ? `failed to start · ${kind}` : "failed to start";
  }
  return null;
}

function nonNegativeInt(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) && value >= 0
    ? Math.trunc(value)
    : null;
}

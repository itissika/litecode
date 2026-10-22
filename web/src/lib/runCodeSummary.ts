/**
 * One-line human summary for `run_code`'s persisted UI data.
 *
 * Wire shape (subset):
 * `{ succeeded, failed, duration_ms, by_tool, ok, failure_kind?, unsettled? }`
 * — never derived from the model-facing content text.
 */
export interface RunCodeChip {
  key: string;
  label: string;
  count: number;
  kind: "tool" | "failed" | "unsettled";
}

/** Aggregate chips for the run_code header (tool totals, failed, unsettled). */
export function runCodeChips(data: unknown): RunCodeChip[] {
  if (data == null || typeof data !== "object" || Array.isArray(data)) {
    return [];
  }
  const rec = data as Record<string, unknown>;
  const chips: RunCodeChip[] = [];

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
    for (const entry of entries) {
      chips.push({
        key: "tool:" + entry.tool,
        label: entry.tool,
        count: entry.count,
        kind: "tool",
      });
    }
  }

  const failed = nonNegativeInt(rec.failed) ?? 0;
  if (failed > 0) {
    chips.push({ key: "failed", label: "failed", count: failed, kind: "failed" });
  }

  const unsettled = nonNegativeInt(rec.unsettled) ?? 0;
  if (unsettled > 0) {
    chips.push({
      key: "unsettled",
      label: "unsettled",
      count: unsettled,
      kind: "unsettled",
    });
  }

  // A program that never started has no counts, only a failure kind.
  if (chips.length === 0 && rec.ok === false) {
    const kind = typeof rec.failure_kind === "string" ? rec.failure_kind : null;
    chips.push({
      key: "start-failed",
      label: kind ? "failed to start · " + kind : "failed to start",
      count: 0,
      kind: "failed",
    });
  }
  return chips;
}

export function formatRunCodeData(data: unknown): string | null {
  const chips = runCodeChips(data);
  if (chips.length === 0) return null;
  return chips
    .map((chip) => {
      if (chip.kind === "unsettled") return chip.count + " unsettled";
      return chip.count > 0 ? chip.label + " ×" + chip.count : chip.label;
    })
    .join(" · ");
}

function nonNegativeInt(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) && value >= 0
    ? Math.trunc(value)
    : null;
}

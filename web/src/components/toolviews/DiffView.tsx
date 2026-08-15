import { useState } from "react";
import { GitDiffIcon } from "@phosphor-icons/react";

type DiffLine = { type: "context" | "add" | "remove"; text: string };

/** Line-level (LCS) diff between two strings, rendered as a unified hunk. */
export function computeLineDiff(oldStr: string, newStr: string): DiffLine[] {
  const a = oldStr.split("\n");
  const b = newStr.split("\n");
  const n = a.length;
  const m = b.length;

  // LCS length table.
  const dp: number[][] = Array.from({ length: n + 1 }, () =>
    new Array(m + 1).fill(0),
  );
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      dp[i][j] =
        a[i] === b[j] ? dp[i + 1][j + 1] + 1 : Math.max(dp[i + 1][j], dp[i][j + 1]);
    }
  }

  const out: DiffLine[] = [];
  let i = 0;
  let j = 0;
  while (i < n && j < m) {
    if (a[i] === b[j]) {
      out.push({ type: "context", text: a[i] });
      i++;
      j++;
    } else if (dp[i + 1][j] >= dp[i][j + 1]) {
      out.push({ type: "remove", text: a[i] });
      i++;
    } else {
      out.push({ type: "add", text: b[j] });
      j++;
    }
  }
  while (i < n) out.push({ type: "remove", text: a[i++] });
  while (j < m) out.push({ type: "add", text: b[j++] });
  return out;
}

const ROW_CLASS: Record<DiffLine["type"], string> = {
  context: "text-(--_dk-text-muted)",
  add: "bg-(--_dk-emerald-500)/10 text-(--_dk-emerald-500)",
  remove: "bg-(--_dk-red-500)/10 text-(--_dk-red-500)",
};

const PREFIX: Record<DiffLine["type"], string> = {
  context: " ",
  add: "+",
  remove: "-",
};

/**
 * Above this many diff lines, collapse to a one-line summary (clipped) that can
 * be expanded into the scrollable full diff. Keeps very large writes/edits from
 * blowing up the card — matching the "summary-clip oversized output" rule.
 */
const DIFF_LINE_LIMIT = 80;

/**
 * Git-style unified diff of two texts. Shared by write (old = "") and edit
 * (old_string/new_string). Oversized diffs are clipped to a summary row.
 */
export function DiffView({ oldText, newText }: { oldText: string; newText: string }) {
  const diff = computeLineDiff(oldText, newText);
  const added = diff.filter((d) => d.type === "add").length;
  const removed = diff.filter((d) => d.type === "remove").length;
  const over = diff.length > DIFF_LINE_LIMIT;
  const [expanded, setExpanded] = useState(!over);

  if (!expanded) {
    return (
      <button
        type="button"
        onClick={() => setExpanded(true)}
        className="flex w-full items-center gap-1.5 rounded-md border border-(--_dk-line-visible) bg-(--_dk-surface-header) px-2 py-1.5 text-dk-xs text-(--_dk-text-muted) hover:brightness-110"
      >
        <GitDiffIcon size={12} aria-hidden />
        <span>Δ {diff.length} lines</span>
        <span className="text-(--_dk-emerald-500)">+{added}</span>
        <span className="text-(--_dk-red-500)">−{removed}</span>
        <span className="ml-auto">Show full diff</span>
      </button>
    );
  }

  return (
    <div className="max-h-60 overflow-auto rounded-md border border-(--_dk-line-visible) font-mono text-dk-sm leading-relaxed">
      {diff.map((line, idx) => (
        <div key={idx} className={`flex px-2 ${ROW_CLASS[line.type]}`}>
          <span className="w-3 shrink-0 select-none opacity-70">
            {PREFIX[line.type]}
          </span>
          <span className="whitespace-pre-wrap break-words">{line.text}</span>
        </div>
      ))}
    </div>
  );
}

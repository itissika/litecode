import type { ReactNode } from "react";
import { X } from "@phosphor-icons/react";
import type { HumanRow } from "../api/types";
import { transcriptMarkKind, type TranscriptMarkKind } from "../api/adapter";
import { AgentMarkdown } from "./AgentMarkdown";
import { WaveText } from "./WaveText";
import { Popover } from "./ui/Popover";

function MarkLine({
  role,
  label,
  children,
  testId,
}: {
  role: "separator" | "status";
  label: string;
  children: ReactNode;
  testId?: string;
}) {
  return (
    <div
      role={role}
      aria-label={label}
      data-testid={testId}
      className="flex items-center gap-1.5 py-1"
    >
      <span className="h-1 w-1 shrink-0 rounded-full bg-(--_dk-text-disabled)" />
      {children}
    </div>
  );
}

/** Clean compact summary for display: drop the "[Conversation summary]" /
 *  "[Aggressive summary]" label prefix and any internal `<system-reminder>`
 *  block, leaving the readable prose. */
export function readableCompactSummary(raw: string): string {
  return raw
    .replace(/^\[(?:Conversation|Aggressive) summary\]\s*/i, "")
    .replace(/<system-reminder>[\s\S]*?<\/system-reminder>/g, "")
    .trim();
}

/** Cut mark between transcript items — not a divider bubble, not summary text.
 *  With a summary the whole line is clickable and opens a floating markdown
 *  panel; without one it stays a plain static cut. */
export function CompactCutMark({ summary }: { summary?: string }) {
  const hasSummary = typeof summary === "string" && summary.trim().length > 0;
  if (!hasSummary) {
    return (
      <MarkLine role="separator" label="Context compacted here">
        <span className="text-dk-2xs text-(--_dk-text-disabled)">compaction point</span>
      </MarkLine>
    );
  }
  return (
    <Popover
      placement="up-right"
      width={440}
      gap={6}
      panelClassName="max-h-96 overflow-y-auto"
      trigger={({ toggle }) => (
        <button
          type="button"
          onClick={toggle}
          aria-label="Show compact summary"
          title="Show compact summary"
          className="group flex w-full cursor-pointer select-none items-center gap-1.5 rounded py-1 text-left transition-colors hover:bg-(--_dk-ix-bg-hover)"
        >
          <span className="h-1 w-1 shrink-0 rounded-full bg-(--_dk-text-disabled)" />
          <span className="text-dk-2xs text-(--_dk-text-disabled) group-hover:text-(--_dk-text-secondary)">
            compaction point
          </span>
        </button>
      )}
    >
      {({ close }) => (
        <div className="p-3">
          <div className="mb-2 flex items-center justify-between gap-2">
            <span className="text-dk-2xs font-medium text-(--_dk-text-secondary)">
              Compaction summary
            </span>
            <button
              type="button"
              onClick={close}
              aria-label="Close compact summary"
              className="flex h-4 w-4 shrink-0 items-center justify-center rounded text-(--_dk-text-muted) transition-colors hover:bg-(--_dk-ix-bg-hover) hover:text-(--_dk-text-secondary)"
            >
              <X size={10} weight="bold" />
            </button>
          </div>
          <AgentMarkdown text={readableCompactSummary(summary)} />
        </div>
      )}
    </Popover>
  );
}

export function JobExitMark() {
  return (
    <MarkLine role="status" label="Background terminal exited">
      <span className="text-dk-2xs text-(--_dk-text-disabled)">background terminal exited</span>
    </MarkLine>
  );
}

/** Transient line while a compaction runs; replaced by CompactCutMark when the row lands. */
export function CompactingMark() {
  return (
    <MarkLine role="status" label="Compacting context" testId="compacting-now">
      <WaveText text="compacting…" className="text-dk-2xs" />
    </MarkLine>
  );
}

export function TranscriptMark({ kind, summary }: { kind: TranscriptMarkKind; summary?: string }) {
  switch (kind) {
    case "compact_cut":
      return <CompactCutMark summary={summary} />;
    case "job_exit":
      return <JobExitMark />;
  }
}

export function TranscriptMarkForRow({ row }: { row: HumanRow }) {
  const kind = transcriptMarkKind(row);
  if (!kind) return null;
  return (
    <TranscriptMark
      kind={kind}
      summary={kind === "compact_cut" && "summary" in row.body ? String(row.body.summary) : undefined}
    />
  );
}

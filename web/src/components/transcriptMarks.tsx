import type { ReactNode } from "react";
import type { HumanRow } from "../api/types";
import { transcriptMarkKind, type TranscriptMarkKind } from "../api/adapter";
import { WaveText } from "./WaveText";

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

/** Cut mark between transcript items — not a divider bubble, not summary text. */
export function CompactCutMark() {
  return (
    <MarkLine role="separator" label="Context compacted here">
      <span className="text-dk-2xs text-(--_dk-text-disabled)">compaction point</span>
    </MarkLine>
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

export function TranscriptMark({ kind }: { kind: TranscriptMarkKind }) {
  switch (kind) {
    case "compact_cut":
      return <CompactCutMark />;
    case "job_exit":
      return <JobExitMark />;
  }
}

export function TranscriptMarkForRow({ row }: { row: HumanRow }) {
  const kind = transcriptMarkKind(row);
  if (!kind) return null;
  return <TranscriptMark kind={kind} />;
}

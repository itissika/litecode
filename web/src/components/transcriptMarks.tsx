import type { ReactNode } from "react";
import { X } from "@phosphor-icons/react";
import type { HumanRow } from "../api/types";
import {
  isMessageItem,
  itemPlainText,
  transcriptMarkKind,
  type TranscriptMarkKind,
} from "../api/adapter";
import { openSubagentPanel } from "../lib/sessionPanelNav";
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

export function JobExitMark({ detail }: { detail?: string }) {
  return (
    <MarkLine role="status" label="Background terminal exited">
      <span className="text-dk-2xs text-(--_dk-text-disabled)">
        {detail ? `background terminal exited · ${detail}` : "background terminal exited"}
      </span>
    </MarkLine>
  );
}

/**
 * One-line subagent completion mark. The report body stays in the log for the
 * agent; humans see a compact cut-style line, optionally opening the child.
 */
export function SubagentExitMark({
  detail,
  childId,
}: {
  detail?: string;
  childId?: string;
}) {
  const label = detail ? `subagent ${detail}` : "subagent settled";
  if (!childId) {
    return (
      <MarkLine role="status" label="Subagent settled" testId="subagent-exit-mark">
        <span className="text-dk-2xs text-(--_dk-text-disabled)">{label}</span>
      </MarkLine>
    );
  }
  return (
    <button
      type="button"
      data-testid="subagent-exit-mark"
      aria-label="Subagent settled"
      onClick={() => openSubagentPanel(childId)}
      className="group flex w-full cursor-pointer select-none items-center gap-1.5 rounded py-1 text-left transition-colors hover:bg-(--_dk-ix-bg-hover)"
    >
      <span className="h-1 w-1 shrink-0 rounded-full bg-(--_dk-text-disabled)" />
      <span className="text-dk-2xs text-(--_dk-text-disabled) group-hover:text-(--_dk-text-secondary)">
        {label}
      </span>
    </button>
  );
}

export function subagentExitDetail(text: string): { detail: string; childId?: string } {
  const ids = [...text.matchAll(/^child_session_id: (\S+)/gm)].map((match) => match[1]!);
  const agents = [...text.matchAll(/^agent: (\S+)/gm)].map((match) => match[1]!);
  const reasons = [...text.matchAll(/^reason: (\S+)/gm)].map((match) => match[1]!);
  const settled = /^settled: (\d+)/m.exec(text);
  const n = settled ? Number(settled[1]) : ids.length || 1;
  const who = n === 1 ? agents[0] || (ids[0] ? ids[0].slice(0, 8) : undefined) : undefined;
  const reason = n === 1 && reasons.length === 1 ? reasons[0] : undefined;
  const parts: string[] = n > 1 ? [`${n} settled`] : ["settled"];
  if (who) parts.push(who);
  if (reason) parts.push(reason);
  return { detail: parts.join(" · "), childId: ids[0] };
}

/**
 * Exit detail carried by a background-terminal reminder body, e.g.
 * `Background bash bg_a exited with code 3.` → `bg_a · exit code 3`, or the
 * user-Kill variant → `bg_a · stopped by user (Kill)`. `undefined` when the
 * body carries no recognizable exit line (nothing extra to show).
 */
export function jobExitDetail(text: string): string | undefined {
  const exited = /^Background bash (\S+) exited with code (-?\d+)\.$/m.exec(text);
  if (exited) return `${exited[1]} · exit code ${exited[2]}`;
  const stopped = /^The user stopped background bash (\S+) \(Kill\)\.$/m.exec(text);
  if (stopped) return `${stopped[1]} · stopped by user (Kill)`;
  return undefined;
}

/** Transient line while a compaction runs; replaced by CompactCutMark when the row lands. */
export function CompactingMark() {
  return (
    <MarkLine role="status" label="Compacting context" testId="compacting-now">
      <WaveText text="compacting…" className="text-dk-2xs" />
    </MarkLine>
  );
}

export function TranscriptMark({
  kind,
  summary,
  detail,
  childId,
}: {
  kind: TranscriptMarkKind;
  summary?: string;
  detail?: string;
  childId?: string;
}) {
  switch (kind) {
    case "compact_cut":
      return <CompactCutMark summary={summary} />;
    case "job_exit":
      return <JobExitMark detail={detail} />;
    case "subagent_exit":
      return <SubagentExitMark detail={detail} childId={childId} />;
  }
}

export function TranscriptMarkForRow({ row }: { row: HumanRow }) {
  const kind = transcriptMarkKind(row);
  if (!kind) return null;
  const text =
    row.kind === "reminder/job_exit" && isMessageItem(row.body)
      ? itemPlainText(row.body)
      : "";
  const sub = kind === "subagent_exit" ? subagentExitDetail(text) : undefined;
  return (
    <TranscriptMark
      kind={kind}
      summary={kind === "compact_cut" && "summary" in row.body ? String(row.body.summary) : undefined}
      detail={
        kind === "job_exit"
          ? jobExitDetail(text)
          : kind === "subagent_exit"
            ? sub?.detail
            : undefined
      }
      childId={sub?.childId}
    />
  );
}

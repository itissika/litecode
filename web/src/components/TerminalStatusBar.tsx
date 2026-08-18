import { TerminalIcon } from "@phosphor-icons/react";
import { useRef } from "react";

import { useBashStore } from "../stores/bashStore";
import { composerCardClass } from "./composerCard";

/** Session-scoped count of alive agent bash jobs. Click cycles to each live view. */
export function TerminalStatusBar({
  sessionId,
  onRevealBash,
}: {
  sessionId: string;
  onRevealBash?: (callId: string) => void;
}) {
  const jobs = useBashStore((s) => s.bySession.get(sessionId)?.jobs ?? EMPTY_JOBS);
  const cursor = useRef(0);

  if (jobs.length <= 0) return null;

  return (
    <button
      type="button"
      className={`${composerCardClass} flex h-[30px] shrink-0 cursor-pointer items-center gap-1.5 px-3 text-xs text-(--_dk-text-secondary)`}
      aria-label={`${jobs.length} active terminal${jobs.length === 1 ? "" : "s"}`}
      onClick={() => {
        if (jobs.length === 0) return;
        const index = cursor.current % jobs.length;
        cursor.current = index + 1;
        const callId = jobs[index]?.call_id;
        if (callId) onRevealBash?.(callId);
      }}
    >
      <TerminalIcon
        size={14}
        weight="fill"
        aria-hidden
        className="terminal-status-icon shrink-0 text-(--_dk-text-secondary)"
      />
      <span className="font-mono text-dk-xs tabular-nums text-(--_dk-text-muted)">
        ×{jobs.length}
      </span>
    </button>
  );
}

const EMPTY_JOBS: import("../api/types").BashJob[] = [];

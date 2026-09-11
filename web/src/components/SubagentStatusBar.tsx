import { useEffect, useRef, useState } from "react";
import { UsersIcon } from "@phosphor-icons/react";

import type { SubagentJob } from "../api/types";
import { useSubagentStore } from "../stores/subagentStore";
import { composerCardClass } from "./composerCard";
import { useChipEntrance } from "./useChipEntrance";

/** Only surface the chip once a worker has been running continuously this long —
 *  launches that fail instantly never flash the dock. */
const SHOW_DELAY_MS = 1000;

/** Once shown, the chip stays up at least this long after the last worker ends —
 *  a worker that finishes right after surfacing would otherwise flash it. */
const MIN_VISIBLE_MS = 1000;

/** Session-scoped count of running subagent workers. Display-only. */
export function SubagentStatusBar({ sessionId }: { sessionId: string }) {
  const jobs = useSubagentStore((s) => s.bySession.get(sessionId)?.jobs ?? EMPTY_JOBS);
  const aliveRef = useRef(false);
  // `visible` = should be shown (debounced entry, hold-over exit); `mounted` /
  // `open` drive the entrance/exit animation via useChipEntrance.
  const [visible, setVisible] = useState(false);

  const alive = jobs.length > 0;
  aliveRef.current = alive;

  // Same debounce/hold semantics as TerminalStatusBar: entry waits SHOW_DELAY_MS
  // of continuous work; exit holds MIN_VISIBLE_MS after the count empties.
  useEffect(() => {
    if (alive) {
      if (visible) return;
      const timer = window.setTimeout(() => {
        if (aliveRef.current) setVisible(true);
      }, SHOW_DELAY_MS);
      return () => window.clearTimeout(timer);
    }
    if (!visible) return;
    const timer = window.setTimeout(() => {
      if (!aliveRef.current) setVisible(false);
    }, MIN_VISIBLE_MS);
    return () => window.clearTimeout(timer);
  }, [alive, visible]);

  const { mounted, open } = useChipEntrance(visible);

  if (!mounted) return null;

  return (
    <div
      className={`dock-chip ${composerCardClass} flex h-[30px] shrink-0 items-center gap-1.5 overflow-hidden px-3 text-xs text-(--_dk-text-secondary) ${open ? "is-open" : ""} ${alive ? "" : "is-empty"}`}
      aria-label={`${jobs.length} running subagent${jobs.length === 1 ? "" : "s"}`}
    >
      <UsersIcon
        size={14}
        weight="fill"
        aria-hidden
        className={`shrink-0 text-(--_dk-text-secondary) ${alive ? "subagent-status-icon" : ""}`}
      />
      <span className="font-mono text-dk-xs tabular-nums text-(--_dk-text-muted)">
        ×{jobs.length}
      </span>
    </div>
  );
}

const EMPTY_JOBS: SubagentJob[] = [];

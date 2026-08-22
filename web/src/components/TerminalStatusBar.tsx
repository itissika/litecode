import { TerminalIcon } from "@phosphor-icons/react";
import { useEffect, useRef, useState } from "react";

import { useBashStore } from "../stores/bashStore";
import { composerCardClass } from "./composerCard";
import { useChipEntrance } from "./useChipEntrance";

/** Only surface the chip once a job has been running continuously this long —
 *  quick bash calls (most of them) never flash the dock. */
const SHOW_DELAY_MS = 1000;

/** Once shown, the chip stays up at least this long after the last job ends —
 *  a bash call that finishes right after surfacing would otherwise flash it. */
const MIN_VISIBLE_MS = 1000;

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
  const aliveRef = useRef(false);
  // `visible` = should be shown (debounced entry, hold-over exit); `mounted` /
  // `open` drive the entrance/exit animation via useChipEntrance.
  const [visible, setVisible] = useState(false);

  const alive = jobs.length > 0;
  aliveRef.current = alive;

  // Entry is debounced: a job must run continuously for SHOW_DELAY_MS before
  // the chip appears, so instant calls never surface it. If the chip is
  // already up (a previous long job surfaced it, or we're inside the hold
  // grace), a new job keeps it up without re-debouncing. Exit holds the chip
  // MIN_VISIBLE_MS after the count empties; repeats of the empty snapshot do
  // NOT extend the hold, and a new job during the hold cancels the timer.
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
    <button
      type="button"
      disabled={!alive}
      className={`dock-chip ${composerCardClass} flex h-[30px] shrink-0 cursor-pointer items-center gap-1.5 overflow-hidden px-3 text-xs text-(--_dk-text-secondary) disabled:cursor-default ${open ? "is-open" : ""} ${alive ? "" : "is-empty"}`}
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
        className={`shrink-0 text-(--_dk-text-secondary) ${alive ? "terminal-status-icon" : ""}`}
      />
      <span className="font-mono text-dk-xs tabular-nums text-(--_dk-text-muted)">
        ×{jobs.length}
      </span>
    </button>
  );
}

const EMPTY_JOBS: import("../api/types").BashJob[] = [];

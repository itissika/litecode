import { TerminalIcon } from "@phosphor-icons/react";
import { useEffect, useRef, useState } from "react";

import { useBashStore } from "../stores/bashStore";
import { composerCardClass } from "./composerCard";
import { useChipEntrance } from "./useChipEntrance";

/** Once shown, the chip stays up at least this long after the last job ends —
 *  a bash call that finishes almost instantly would otherwise flash it. */
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
  // `visible` = should be shown (hold-over aware); `mounted` / `open` drive the
  // entrance/exit animation via useChipEntrance.
  const [visible, setVisible] = useState(false);

  const alive = jobs.length > 0;
  aliveRef.current = alive;

  // Hold-over: whenever the count empties, keep the chip visible another
  // MIN_VISIBLE_MS before dismissing it — regardless of how long the jobs ran.
  // Repeats of the empty snapshot do NOT extend the hold (effect only re-runs
  // on transitions), and a new job during the hold cancels the timer.
  useEffect(() => {
    if (alive) {
      setVisible(true);
      return;
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

import { TerminalIcon } from "@phosphor-icons/react";

import { useBashStore } from "../stores/bashStore";
import { composerCardClass } from "./composerCard";

/** Session-scoped count of alive agent bash jobs — display only, no interaction. */
export function TerminalStatusBar({ sessionId }: { sessionId: string }) {
  const count = useBashStore(
    (s) => s.bySession.get(sessionId)?.jobs.length ?? 0,
  );

  if (count <= 0) return null;

  return (
    <div
      className={`${composerCardClass} flex h-[30px] shrink-0 items-center gap-1.5 px-3 text-xs text-(--_dk-text-secondary)`}
      aria-label={`${count} active terminal${count === 1 ? "" : "s"}`}
    >
      <TerminalIcon
        size={14}
        weight="fill"
        aria-hidden
        className="terminal-status-icon shrink-0 text-(--_dk-text-secondary)"
      />
      <span className="font-mono text-dk-xs tabular-nums text-(--_dk-text-muted)">
        ×{count}
      </span>
    </div>
  );
}

import { useEffect, useState } from "react";

import { WaveText } from "../WaveText";
import { formatElapsed } from "../../lib/bashLive";
import { useBashStore } from "../../stores/bashStore";
import type { ToolViewProps } from "./registry";

const waitLineClass = "font-mono text-dk-sm";

/**
 * Auxiliary wait_shell view: countdown while this call is waiting, then
 * "waited". Live output stays on the bash card.
 */
export function WaitShellToolView({ call_id, sessionId, output, status }: ToolViewProps) {
  const waiter = useBashStore((s) => {
    if (!sessionId || !call_id) return undefined;
    return s.bySession.get(sessionId)?.waits.find((w) => w.call_id === call_id);
  });
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    if (!waiter) return;
    const t = window.setInterval(() => setNow(Date.now()), 250);
    return () => window.clearInterval(t);
  }, [waiter]);

  if (waiter) {
    const label =
      waiter.deadline_ms != null
        ? formatElapsed(waiter.deadline_ms - now)
        : formatElapsed(now - waiter.started_at_ms);
    return (
      <div className={waitLineClass} data-testid="wait-elapsed">
        <WaveText text={`wait ${label}`} />
      </div>
    );
  }

  if (status === "failed") {
    return (
      <div className={`${waitLineClass} text-(--_dk-red-500)`}>wait failed</div>
    );
  }

  if (output) {
    return (
      <div className={`${waitLineClass} text-(--_dk-text-muted)`}>waited</div>
    );
  }

  return (
    <div className={waitLineClass} data-testid="wait-pending">
      <WaveText text="waiting…" />
    </div>
  );
}

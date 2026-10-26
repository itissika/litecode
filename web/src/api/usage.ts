import { useEffect, useState } from "react";

import { useConnectionStore } from "../stores/connectionStore";
import type { SessionUsageSeries, WorkspaceUsageReport } from "./types";

/** Bills the chart asks for. The server clamps this to 1..400. */
export const USAGE_SERIES_LIMIT = 200;

/**
 * Provider bills for one session: the last `USAGE_SERIES_LIMIT` requests plus
 * whole-session totals. Read-only — every number is server-owned, and both
 * halves come from the same rows.
 */
export function fetchSessionUsage(
  sessionId: string,
  limit: number = USAGE_SERIES_LIMIT,
): Promise<SessionUsageSeries> {
  return useConnectionStore.getState().sendRpc<SessionUsageSeries>(
    "session/usage",
    { session_id: sessionId, limit },
  );
}

/** What a usage read knows right now. */
export type SessionUsage = {
  /** Always this session's series, never a late reply for another one. */
  series: SessionUsageSeries | null;
  failed: boolean;
};

/**
 * Provider bills for one session, refetched whenever `refreshKey` changes.
 *
 * A series that belongs to another session is never handed out, so a late reply
 * after switching sessions cannot paint the wrong history.
 */
export function useSessionUsage(
  sessionId: string,
  refreshKey: string | number,
): SessionUsage {
  const [series, setSeries] = useState<SessionUsageSeries | null>(null);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    let cancelled = false;
    setFailed(false);
    fetchSessionUsage(sessionId)
      .then((next) => {
        if (cancelled) return;
        setSeries(next);
      })
      .catch(() => {
        if (cancelled) return;
        setSeries(null);
        setFailed(true);
      });
    return () => {
      cancelled = true;
    };
  }, [sessionId, refreshKey]);

  const current = series && series.session_id === sessionId ? series : null;
  return { series: current, failed: failed && current === null };
}

/**
 * Whole-workspace bills, once per open. There is no per-request tick here: the
 * popover unmounts when closed, so every open reads current truth.
 */
export function fetchWorkspaceUsage(): Promise<WorkspaceUsageReport> {
  return useConnectionStore.getState().sendRpc<WorkspaceUsageReport>(
    "workspace/usage",
    {},
  );
}

export function useWorkspaceUsage(): {
  report: WorkspaceUsageReport | null;
  failed: boolean;
} {
  const [report, setReport] = useState<WorkspaceUsageReport | null>(null);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    let cancelled = false;
    fetchWorkspaceUsage()
      .then((next) => {
        if (cancelled) return;
        setReport(next);
      })
      .catch(() => {
        if (cancelled) return;
        setReport(null);
        setFailed(true);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  return { report, failed };
}

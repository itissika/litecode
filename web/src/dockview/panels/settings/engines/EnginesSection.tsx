import { useCallback, useEffect, useRef, useState } from "react";

import {
  getEnginesDetail,
  type EnginesDetail,
} from "../../../../api/workspace";
import {
  setEngineDetailPolling,
  useEngineStore,
} from "../../../../stores/engineStore";
import { EngineView } from "./EngineView";
import { retrievalBusy } from "./retrievalBar";

export const POLL_FAST_MS = 1000;
export const POLL_IDLE_MS = 5000;
/** After a refresh/rebuild click: poll at 1s for this many cycles. */
export const FAST_CYCLES_AFTER_REFRESH = 3;
import { ENGINES_CHANGED_EVENT } from "./embedInstall";
import { EnginesSkeleton } from "../../../../components/ui/Skeleton";
import { SettingsPageShell } from "../shared";

export function EnginesSection() {
  const [detail, setDetail] = useState<EnginesDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  /** Bumps after every fetch settles, so polling re-arms even when the payload is identical. */
  const [pollTick, setPollTick] = useState(0);
  useEffect(() => {
    setEngineDetailPolling(true);
    return () => setEngineDetailPolling(false);
  }, []);
  const refresh = useCallback(() => {
    void getEnginesDetail()
      .then((next) => {
        setDetail(next);
        setError(null);
        useEngineStore.getState().applyFromDetail(next);
      })
      .catch((e) => setError(e instanceof Error ? e.message : String(e)))
      .finally(() => setPollTick((t) => t + 1));
  }, []);
  useEffect(() => {
    refresh();
    // Embed model installed (from any view or Open Remote): refetch so the
    // "model missing" state clears without reopening Settings.
    window.addEventListener(ENGINES_CHANGED_EVENT, refresh);
    return () => window.removeEventListener(ENGINES_CHANGED_EVENT, refresh);
  }, [refresh]);
  // WP-136: poll the whole time Settings is visible — 1s while the backend is
  // refreshing / an engine warms / just after a refresh click, else ~5s.
  // Fast cycles after a click only tighten polling; the bar follows the backend.
  const fastCycles = useRef(0);
  const [pollNonce, setPollNonce] = useState(0);
  const onRefreshRequested = useCallback(() => {
    fastCycles.current = FAST_CYCLES_AFTER_REFRESH;
    setPollNonce((n) => n + 1); // re-arm at the fast interval now
  }, []);
  const [pageVisible, setPageVisible] = useState(
    () => typeof document === "undefined" || document.visibilityState !== "hidden",
  );
  useEffect(() => {
    const onVis = () => {
      const visible = document.visibilityState !== "hidden";
      setPageVisible(visible);
      if (visible) refresh();
    };
    document.addEventListener("visibilitychange", onVis);
    return () => document.removeEventListener("visibilitychange", onVis);
  }, [refresh]);
  useEffect(() => {
    if (!detail || !pageVisible) return;
    const fast =
      fastCycles.current > 0 ||
      retrievalBusy(detail.retrieval) ||
      detail.retrieval.usable === "warming" ||
      detail.lsp.usable === "warming";
    const timer = window.setTimeout(
      () => {
        if (fastCycles.current > 0) fastCycles.current -= 1;
        refresh();
      },
      fast ? POLL_FAST_MS : POLL_IDLE_MS,
    );
    return () => window.clearTimeout(timer);
  }, [detail, refresh, pageVisible, pollNonce, pollTick]);

  if (error) {
    return (
      <SettingsPageShell title="Engines">
        <p className="text-sm text-(--_dk-red-500)">{error}</p>
      </SettingsPageShell>
    );
  }
  if (!detail) {
    return (
      <SettingsPageShell title="Engines">
        <EnginesSkeleton />
      </SettingsPageShell>
    );
  }
  return (
    <SettingsPageShell title="Engines">
      <EngineView
        detail={detail}
        onChanged={refresh}
        onRefreshRequested={onRefreshRequested}
      />
    </SettingsPageShell>
  );
}

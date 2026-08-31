import { useCallback, useEffect, useState } from "react";

import { getEnginesDetail, type EnginesDetail } from "../../../../api/workspace";
import { setEngineDetailPolling, useEngineStore } from "../../../../stores/engineStore";
import { EngineView } from "./EngineView";
import { EnginesSkeleton } from "../../../../components/ui/Skeleton";
import { SettingsPageShell } from "../shared";

export function EnginesSection() {
  const [detail, setDetail] = useState<EnginesDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
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
      .catch((e) => setError(e instanceof Error ? e.message : String(e)));
  }, []);
  useEffect(() => { refresh(); }, [refresh]);
  useEffect(() => {
    if (!detail) return;
    const indexStatus = detail.retrieval.index.status;
    const busy =
      detail.retrieval.usable === "warming" ||
      detail.lsp.usable === "warming" ||
      indexStatus === "building" ||
      indexStatus === "refreshing";
    if (!busy) return;
    const timer = window.setInterval(refresh, 1000);
    return () => window.clearInterval(timer);
  }, [detail, refresh]);

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
      <EngineView detail={detail} onChanged={refresh} />
    </SettingsPageShell>
  );
}

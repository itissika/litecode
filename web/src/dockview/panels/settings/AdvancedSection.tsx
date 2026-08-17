import { useEffect, useState } from "react";

import { useSettingsStore } from "../../../stores/settingsStore";
import { Select } from "../../../components/ui/Select";
import {
  FieldLabel,
  TextInput,
  SectionHeader,
  SettingsPageShell,
} from "./shared";
import { isPersistBusy, useSettingsPersist } from "./persist";

const LOG_LEVELS = ["trace", "debug", "info", "warn", "error"] as const;

type AdvancedDraft = { logLevel: string; searchEndpoint: string };

export function AdvancedSection() {
  const log = useSettingsStore((s) => s.log);
  const websearch = useSettingsStore((s) => s.websearch);
  const saveBlocked = useSettingsStore((s) => s.isSaveBlocked());
  const persistStatus = useSettingsStore((s) => s.persistStatus);
  const setPersistStatus = useSettingsStore((s) => s.setPersistStatus);
  const saveLog = useSettingsStore((s) => s.saveLog);
  const saveWebSearch = useSettingsStore((s) => s.saveWebSearch);
  const [draft, setDraft] = useState<AdvancedDraft>({
    logLevel: "info",
    searchEndpoint: "",
  });

  useEffect(() => {
    if (isPersistBusy(persistStatus)) return;
    setDraft({
      logLevel: log?.level ?? "info",
      searchEndpoint: websearch?.search_endpoint ?? "",
    });
  }, [log, websearch, persistStatus]);

  useSettingsPersist(draft, {
    debounceMs: 400,
    setStatus: setPersistStatus,
    serialize: (d) => ({
      ok: {
        logLevel: d.logLevel || null,
        searchEndpoint: d.searchEndpoint.trim() || undefined,
      },
    }),
    commit: async (p) => {
      const snap = useSettingsStore.getState();
      const prevEndpoint = snap.websearch?.search_endpoint ?? "";
      const prevLevel = snap.log?.level ?? "info";
      if ((p.searchEndpoint ?? "") !== prevEndpoint) {
        await saveWebSearch({ search_endpoint: p.searchEndpoint });
      }
      if ((p.logLevel ?? "info") !== prevLevel) {
        await saveLog(p.logLevel);
      }
    },
    revert: () => {
      const snap = useSettingsStore.getState();
      setDraft({
        logLevel: snap.log?.level ?? "info",
        searchEndpoint: snap.websearch?.search_endpoint ?? "",
      });
    },
  });

  return (
    <SettingsPageShell title="Advanced">
      <div className="space-y-6">
        <div className="settings-card space-y-3 p-4">
          <SectionHeader title="Web search" />
          <FieldLabel>Exa MCP URL (optional override)</FieldLabel>
          <TextInput
            value={draft.searchEndpoint}
            onChange={(e) => setDraft((d) => ({ ...d, searchEndpoint: e.target.value }))}
            placeholder="https://mcp.exa.ai/mcp"
            disabled={saveBlocked}
          />
        </div>

        <div className="settings-card space-y-3 p-4">
          <SectionHeader title="Log level" />
          <FieldLabel>Level</FieldLabel>
          <Select
            value={draft.logLevel}
            onChange={(logLevel) => setDraft((d) => ({ ...d, logLevel }))}
            options={LOG_LEVELS.map((level) => ({ value: level, label: level }))}
            disabled={saveBlocked}
            className="w-full"
          />
        </div>
      </div>
    </SettingsPageShell>
  );
}

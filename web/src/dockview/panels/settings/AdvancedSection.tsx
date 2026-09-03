import { useEffect, useState } from "react";

import { useSettingsStore } from "../../../stores/settingsStore";
import { Select } from "../../../components/ui/Select";
import {
  FieldLabel,
  TextInput,
  SectionHeader,
  SettingsPageShell,
  useSettingsSaveBlocked,
} from "./shared";
import { shouldHydrateDraftFromStore, useDocPersist, useSettingsPersist } from "./persist";

const LOG_LEVELS = ["trace", "debug", "info", "warn", "error"] as const;

function snapshotLogLevel(): string {
  return useSettingsStore.getState().log?.level ?? "info";
}

function snapshotSearchEndpoint(): string {
  return useSettingsStore.getState().websearch?.search_endpoint ?? "";
}

export function AdvancedSection() {
  const log = useSettingsStore((s) => s.log);
  const websearch = useSettingsStore((s) => s.websearch);
  const saveBlocked = useSettingsSaveBlocked();
  const logPersist = useDocPersist("log");
  const searchPersist = useDocPersist("websearch");
  const saveLog = useSettingsStore((s) => s.saveLog);
  const saveWebSearch = useSettingsStore((s) => s.saveWebSearch);
  const [logLevel, setLogLevel] = useState(snapshotLogLevel);
  const [searchEndpoint, setSearchEndpoint] = useState(snapshotSearchEndpoint);

  useEffect(() => {
    if (shouldHydrateDraftFromStore(logPersist.persistStatus)) {
      setLogLevel(log?.level ?? "info");
    }
    if (shouldHydrateDraftFromStore(searchPersist.persistStatus)) {
      setSearchEndpoint(websearch?.search_endpoint ?? "");
    }
  }, [log, websearch, logPersist.persistStatus, searchPersist.persistStatus]);

  useSettingsPersist(searchEndpoint, {
    debounceMs: 400,
    setStatus: searchPersist.setPersistStatus,
    serialize: (value) => ({ ok: value.trim() || undefined }),
    commit: (search_endpoint) => saveWebSearch({ search_endpoint }),
    revert: () => setSearchEndpoint(snapshotSearchEndpoint()),
  });

  useSettingsPersist(logLevel, {
    debounceMs: 400,
    setStatus: logPersist.setPersistStatus,
    serialize: (value) => ({ ok: value || null }),
    commit: (level) => saveLog(level),
    revert: () => setLogLevel(snapshotLogLevel()),
  });

  return (
    <SettingsPageShell title="Advanced">
      <div className="space-y-6">
        <div className="settings-card space-y-3 p-4">
          <SectionHeader title="Web search" />
          <FieldLabel>Exa MCP URL (optional override)</FieldLabel>
          <TextInput
            value={searchEndpoint}
            onChange={(e) => setSearchEndpoint(e.target.value)}
            placeholder="https://mcp.exa.ai/mcp"
            disabled={saveBlocked}
          />
        </div>

        <div className="settings-card space-y-3 p-4">
          <SectionHeader title="Log level" />
          <FieldLabel>Level</FieldLabel>
          <Select
            value={logLevel}
            onChange={setLogLevel}
            options={LOG_LEVELS.map((level) => ({ value: level, label: level }))}
            disabled={saveBlocked}
            className="w-full"
          />
        </div>
      </div>
    </SettingsPageShell>
  );
}

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

function snapshotApiKey(): string {
  return useSettingsStore.getState().websearch?.api_key ?? "";
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
  const [apiKey, setApiKey] = useState(snapshotApiKey);

  useEffect(() => {
    if (shouldHydrateDraftFromStore(logPersist.persistStatus)) {
      setLogLevel(log?.level ?? "info");
    }
    if (shouldHydrateDraftFromStore(searchPersist.persistStatus)) {
      setApiKey(websearch?.api_key ?? "");
    }
  }, [log, websearch, logPersist.persistStatus, searchPersist.persistStatus]);

  useSettingsPersist(apiKey, {
    debounceMs: 400,
    setStatus: searchPersist.setPersistStatus,
    serialize: (value) => ({ ok: value }),
    commit: (api_key) => saveWebSearch({ api_key }),
    revert: () => setApiKey(snapshotApiKey()),
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
          <p className="text-xs text-(--_dk-text-muted)">
            Uses Exa hosted search. Paste an Exa API key for higher limits; leave empty for the
            anonymous free tier, or set <code className="font-mono">EXA_API_KEY</code> in the
            environment.
          </p>
          <FieldLabel>Exa API key</FieldLabel>
          <TextInput
            type="password"
            autoComplete="off"
            value={apiKey}
            onChange={(e) => setApiKey(e.target.value)}
            placeholder="optional"
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

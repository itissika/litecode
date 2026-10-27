import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { useSettingsStore } from "../../../stores/settingsStore";
import { Select } from "../../../components/ui/Select";
import {
  FieldLabel,
  TextInput,
  SectionHeader,
  SettingsPageShell,
  useSettingsSaveBlocked,
} from "./shared";
import {
  shouldHydrateDraftFromStore,
  useDocPersist,
  useSettingsPersist,
} from "./persist";

const LOG_LEVELS = ["trace", "debug", "info", "warn", "error"] as const;

function snapshotLogLevel(): string {
  return useSettingsStore.getState().log?.level ?? "info";
}

function snapshotApiKey(): string {
  return useSettingsStore.getState().websearch?.api_key ?? "";
}

export function AdvancedSection() {
  const { t } = useTranslation();
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
    <SettingsPageShell title={t("settings.advanced.title")}>
      <div className="space-y-6">
        <div className="settings-card space-y-3 p-4">
          <SectionHeader title={t("settings.advanced.webSearch")} />
          <p className="text-xs text-(--_dk-text-muted)">
            {t("settings.advanced.exaHint")}
          </p>
          <FieldLabel>{t("settings.advanced.exaKey")}</FieldLabel>
          <TextInput
            type="password"
            autoComplete="off"
            value={apiKey}
            onChange={(e) => setApiKey(e.target.value)}
            placeholder={t("common.optional")}
            disabled={saveBlocked}
          />
        </div>

        <div className="settings-card space-y-3 p-4">
          <SectionHeader title={t("settings.advanced.logLevel")} />
          <FieldLabel>{t("settings.advanced.level")}</FieldLabel>
          <Select
            value={logLevel}
            onChange={setLogLevel}
            options={LOG_LEVELS.map((level) => ({
              value: level,
              label: level,
            }))}
            disabled={saveBlocked}
            className="w-full"
          />
        </div>
      </div>
    </SettingsPageShell>
  );
}

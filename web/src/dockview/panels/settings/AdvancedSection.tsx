import { useEffect, useState, type FormEvent } from "react";

import { useSettingsStore } from "../../../stores/settingsStore";
import { Select } from "../../../components/ui/Select";
import {
  FieldLabel,
  TextInput,
  SectionHeader,
  SettingsPageShell,
} from "./shared";

const LOG_LEVELS = ["trace", "debug", "info", "warn", "error"] as const;

export function AdvancedSection() {
  const log = useSettingsStore((s) => s.log);
  const websearch = useSettingsStore((s) => s.websearch);
  const saving = useSettingsStore((s) => s.saving);
  const saveBlocked = useSettingsStore((s) => s.isSaveBlocked());
  const saveLog = useSettingsStore((s) => s.saveLog);
  const saveWebSearch = useSettingsStore((s) => s.saveWebSearch);
  const [logLevel, setLogLevel] = useState<string>("info");
  const [searchEndpoint, setSearchEndpoint] = useState("");

  useEffect(() => {
    setLogLevel(log?.level ?? "info");
  }, [log]);

  useEffect(() => {
    setSearchEndpoint(websearch?.search_endpoint ?? "");
  }, [websearch]);

  const onSubmit = (e: FormEvent) => {
    e.preventDefault();
    void (async () => {
      await saveWebSearch({
        search_endpoint: searchEndpoint.trim() || undefined,
      });
      await saveLog(logLevel || null);
    })();
  };

  return (
    <SettingsPageShell
      title="Advanced"
      onSubmit={onSubmit}
      save={{ disabled: saveBlocked, saving }}
    >
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

import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import { type McpProbeResult, type ToolScope } from "../../../api/settings";
import { useSettingsStore } from "../../../stores/settingsStore";
import type { McpDefItem } from "../../../stores/settingsDocuments";
import { mergeLayeredMcp } from "../../../stores/settingsDocuments";
import {
  FieldLabel,
  TextArea,
  useSettingsSaveBlocked,
} from "./shared";
import { parseMcpJson } from "./jsonDefinitions";
import { ScopeHint, ScopedDefinitionEditor } from "./ScopedDefinitionEditor";
import {
  flushRegisteredSettings,
  isPersistBusy,
  shouldHydrateDraftFromStore,
  useDocPersist,
  useSettingsPersist,
} from "./persist";

const EMPTY_MCP_JSON = `{
  "id": "filesystem",
  "command": "npx",
  "args": ["-y", "@modelcontextprotocol/server-filesystem", "/path/to/folder"],
  "env": {},
  "transport": { "type": "stdio" },
  "timeout": 60
}`;

function prettyServer(item: McpDefItem): string {
  return JSON.stringify(
    {
      id: item.id,
      command: item.command,
      args: item.args ?? [],
      env: item.env ?? {},
      transport: item.transport ?? { type: "stdio" },
      timeout: item.timeout && item.timeout > 0 ? item.timeout : 60,
    },
    null,
    2,
  );
}

export function McpServersSection() {
  const { t } = useTranslation();
  const mcpDefs = useSettingsStore((s) => s.mcpDefs);
  const mcpRuntime = useSettingsStore((s) => s.mcpRuntime);
  const mcpServers = useMemo(
    () => mergeLayeredMcp(mcpDefs, mcpRuntime),
    [mcpDefs, mcpRuntime],
  );
  const saveBlocked = useSettingsSaveBlocked();
  const { persistStatus, setPersistStatus } = useDocPersist("mcp");
  const saveMcpServer = useSettingsStore((s) => s.saveMcpServer);
  const removeMcpServer = useSettingsStore((s) => s.removeMcpServer);
  const startMcp = useSettingsStore((s) => s.startMcpServer);
  const restartMcp = useSettingsStore((s) => s.restartMcpServer);
  const stopMcp = useSettingsStore((s) => s.stopMcpServer);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [selectedScope, setSelectedScope] = useState<ToolScope>("global");
  const [jsonText, setJsonText] = useState(EMPTY_MCP_JSON);
  const [formError, setFormError] = useState<string | null>(null);
  const [isNew, setIsNew] = useState(false);
  const [createScope, setCreateScope] = useState<ToolScope>("global");
  const [probe, setProbe] = useState<McpProbeResult | null>(null);
  const [busy, setBusy] = useState<"start" | "restart" | "stop" | null>(null);

  const globalServers = mcpServers?.global ?? [];
  const workspaceServers = mcpServers?.workspace ?? [];

  const defSource =
    selectedScope === "workspace" ? mcpDefs?.workspace : mcpDefs?.global;

  useEffect(() => {
    if (isNew) return;
    if (!shouldHydrateDraftFromStore(persistStatus)) return;
    const list = defSource ?? [];
    if (selectedId) {
      const found = list.find((s) => s.id === selectedId);
      if (found) {
        setJsonText(prettyServer(found));
        setFormError(null);
        return;
      }
    }
    if (list.length === 0) {
      setSelectedId(null);
      setJsonText(EMPTY_MCP_JSON);
    }
  }, [defSource, selectedId, isNew, persistStatus]);

  useSettingsPersist(jsonText, {
    enabled: !isNew && !!selectedId,
    debounceMs: 400,
    setStatus: setPersistStatus,
    serialize: (text) => parseMcpJson(text, selectedId),
    commit: (p) => saveMcpServer(p.id, p.def, selectedScope),
    revert: () => {
      const defs = useSettingsStore.getState().mcpDefs;
      const found = (
        selectedScope === "workspace" ? defs?.workspace : defs?.global
      )?.find((s) => s.id === selectedId);
      if (found) setJsonText(prettyServer(found));
    },
  });

  const startCreate = (scope: ToolScope) => {
    setCreateScope(scope);
    setSelectedScope(scope);
    setIsNew(true);
    setSelectedId(null);
    setJsonText(EMPTY_MCP_JSON);
    setFormError(null);
    setProbe(null);
  };

  const onCreate = () => {
    setFormError(null);
    const parsed = parseMcpJson(jsonText);
    if ("skip" in parsed) {
      setFormError(t("settings.mcp.jsonRequired"));
      return;
    }
    void (async () => {
      try {
        await saveMcpServer(parsed.ok.id, parsed.ok.def, createScope);
        setIsNew(false);
        setSelectedScope(createScope);
        setSelectedId(parsed.ok.id);
        setPersistStatus("saved");
      } catch {
        // toast already shown
      }
    })();
  };

  const onDelete = (id: string, scope: ToolScope) => {
    if (!id || isNew || saveBlocked) return;
    void (async () => {
      try {
        await removeMcpServer(id, scope);
        setIsNew(false);
        setSelectedId(null);
        setProbe(null);
      } catch {
        // toast already shown
      }
    })();
  };

  const runLifecycle = (
    action: "start" | "restart" | "stop",
    fn: () => Promise<McpProbeResult>,
  ) => {
    if (action !== "stop") {
      const parsed = parseMcpJson(jsonText, isNew ? null : selectedId);
      if ("skip" in parsed) {
        setFormError(t("settings.mcp.fixJsonStart"));
        return;
      }
    }
    setBusy(action);
    setProbe(null);
    void (async () => {
      try {
        if (action !== "stop") {
          await flushRegisteredSettings();
        }
        const result = await fn();
        setProbe(result);
      } catch (err) {
        setProbe({
          ready: false,
          tools: [],
          error: err instanceof Error ? err.message : t("settings.mcp.actionFailed"),
        });
      } finally {
        setBusy(null);
      }
    })();
  };

  const onStart = () => {
    const parsed = parseMcpJson(jsonText, isNew ? null : selectedId);
    if ("skip" in parsed) {
      setFormError(t("settings.mcp.fixJsonStart"));
      return;
    }
    runLifecycle("start", () => startMcp(parsed.ok.id, selectedScope));
  };

  const onRestart = () => {
    const parsed = parseMcpJson(jsonText, isNew ? null : selectedId);
    if ("skip" in parsed) {
      setFormError(t("settings.mcp.fixJsonRestart"));
      return;
    }
    runLifecycle("restart", () => restartMcp(parsed.ok.id, selectedScope));
  };

  const onStop = () => {
    if (!selectedId) return;
    runLifecycle("stop", () => stopMcp(selectedId, selectedScope));
  };

  const editorForm = (
    <div className="settings-card space-y-3 p-4">
      <div>
        <FieldLabel required>{t("settings.mcp.definitionJson")}</FieldLabel>
        <TextArea
          rows={14}
          value={jsonText}
          disabled={saveBlocked}
          onChange={(e) => setJsonText(e.target.value)}
          className="font-mono text-xs"
          spellCheck={false}
        />
      </div>
      <div className="flex flex-wrap items-center gap-2">
        <button
          type="button"
          className="btn-primary btn-sm"
          disabled={saveBlocked || busy !== null || isNew}
          onClick={onStart}
        >
          {busy === "start" ? t("settings.mcp.starting") : t("settings.mcp.start")}
        </button>
        <button
          type="button"
          className="btn btn-sm"
          disabled={saveBlocked || busy !== null || isNew || !selectedId}
          onClick={onRestart}
        >
          {busy === "restart" ? t("settings.mcp.restarting") : t("settings.mcp.restart")}
        </button>
        <button
          type="button"
          className="btn btn-sm"
          disabled={saveBlocked || busy !== null || isNew || !selectedId}
          onClick={onStop}
        >
          {busy === "stop" ? t("settings.mcp.stopping") : t("settings.mcp.stop")}
        </button>
        <span className="text-xs text-(--_dk-text-muted)">
          {t("settings.mcp.startHint")}
        </span>
      </div>
      {probe ? (
        probe.ready ? (
          <p className="text-xs text-(--_dk-text-secondary)">
            {t("settings.mcp.readyTools", {
              tools: probe.tools.length
                ? probe.tools.map((tool) => tool.name).join(", ")
                : t("settings.mcp.noneListed"),
            })}
          </p>
        ) : (
          <p className="text-sm text-(--_dk-red-500)">
            {probe.error || t("settings.mcp.probeFailed")}
          </p>
        )
      ) : null}
      {formError ? (
        <p className="text-sm text-(--_dk-red-500)">{formError}</p>
      ) : null}
    </div>
  );

  return (
    <ScopedDefinitionEditor
      title={t("settings.mcp.title")}
      newLabel={t("settings.mcp.newServer")}
      hint={<ScopeHint bindName="mcp_<id>" />}
      global={globalServers}
      workspace={workspaceServers}
      itemId={(server) => server.id}
      itemSummary={(server) =>
        `${server.status ?? t("settings.mcp.stopped")}${server.command ? ` · ${server.command}` : ""}`
      }
      actionsDisabled={saveBlocked || isPersistBusy(persistStatus)}
      isNew={isNew}
      createScope={createScope}
      selectedScope={selectedScope}
      selectedId={selectedId}
      onCreate={onCreate}
      onStartCreate={startCreate}
      onCancelNew={() => setIsNew(false)}
      onOpen={(scope, id) => {
        void flushRegisteredSettings().then(() => {
          setIsNew(false);
          setSelectedScope(scope);
          setSelectedId(id);
          setProbe(null);
        });
      }}
      onClose={() => {
        void flushRegisteredSettings().then(() => setSelectedId(null));
      }}
      onDelete={onDelete}
      editorForm={editorForm}
    />
  );
}

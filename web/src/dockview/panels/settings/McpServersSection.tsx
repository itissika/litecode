import { useEffect, useMemo, useState } from "react";
import { Plus, Trash } from "@phosphor-icons/react";

import {
  type McpProbeResult,
  type ToolScope,
} from "../../../api/settings";
import { useSettingsStore } from "../../../stores/settingsStore";
import type { McpDefItem } from "../../../stores/settingsDocuments";
import { mergeLayeredMcp } from "../../../stores/settingsDocuments";
import { FoldCard } from "../../../components/FoldCard";
import { Dropdown, dropdownItemClass } from "../../../components/ui/Dropdown";
import { FieldLabel, TextArea, SettingsPageShell, useSettingsSaveBlocked } from "./shared";
import { parseMcpJson } from "./jsonDefinitions";
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
      const found = (selectedScope === "workspace"
        ? defs?.workspace
        : defs?.global
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
      setFormError("Valid JSON with id and stdio command is required");
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
        setFormError("Fix JSON before starting");
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
          error: err instanceof Error ? err.message : "MCP action failed",
        });
      } finally {
        setBusy(null);
      }
    })();
  };

  const onStart = () => {
    const parsed = parseMcpJson(jsonText, isNew ? null : selectedId);
    if ("skip" in parsed) {
      setFormError("Fix JSON before starting");
      return;
    }
    runLifecycle("start", () => startMcp(parsed.ok.id, selectedScope));
  };

  const onRestart = () => {
    const parsed = parseMcpJson(jsonText, isNew ? null : selectedId);
    if ("skip" in parsed) {
      setFormError("Fix JSON before restarting");
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
        <FieldLabel required>Definition (JSON)</FieldLabel>
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
          {busy === "start" ? "Starting…" : "Start"}
        </button>
        <button
          type="button"
          className="btn btn-sm"
          disabled={saveBlocked || busy !== null || isNew || !selectedId}
          onClick={onRestart}
        >
          {busy === "restart" ? "Restarting…" : "Restart"}
        </button>
        <button
          type="button"
          className="btn btn-sm"
          disabled={saveBlocked || busy !== null || isNew || !selectedId}
          onClick={onStop}
        >
          {busy === "stop" ? "Stopping…" : "Stop"}
        </button>
        <span className="text-xs text-(--_dk-text-muted)">
          Keeps a stdio process running. Save JSON, then Start. Config changes need Restart.
        </span>
      </div>
      {probe ? (
        probe.ready ? (
          <p className="text-xs text-(--_dk-text-secondary)">
            Ready — tools: {probe.tools.length ? probe.tools.map((tool) => tool.name).join(", ") : "(none listed)"}
          </p>
        ) : (
          <p className="text-sm text-(--_dk-red-500)">{probe.error || "Probe failed"}</p>
        )
      ) : null}
      {formError ? (
        <p className="text-sm text-(--_dk-red-500)">{formError}</p>
      ) : null}
    </div>
  );

  return (
    <SettingsPageShell
      title="MCP"
      actions={
        <>
          {isNew ? (
            <button
              type="button"
              className="btn-primary btn-sm"
              disabled={saveBlocked || isPersistBusy(persistStatus)}
              onClick={onCreate}
            >
              Create
            </button>
          ) : null}
          <Dropdown
            variant="menu"
            align="right"
            panelClassName="rounded-md"
            trigger={({ open, toggle }) => (
              <button
                type="button"
                className="btn btn-icon"
                disabled={saveBlocked || isPersistBusy(persistStatus)}
                aria-label="New MCP server"
                aria-haspopup="menu"
                aria-expanded={open}
                title="New MCP server"
                onClick={toggle}
              >
                <Plus size={16} />
              </button>
            )}
          >
            <button
              type="button"
              className={dropdownItemClass}
              onClick={() => startCreate("global")}
            >
              Global
            </button>
            <button
              type="button"
              className={dropdownItemClass}
              onClick={() => startCreate("workspace")}
            >
              Workspace
            </button>
          </Dropdown>
        </>
      }
    >
      <div className="space-y-4">
        <p className="text-xs text-(--_dk-text-muted)">
          Register stdio MCP servers. Same id: workspace overrides global. Bind{" "}
          <span className="font-mono">mcp_&lt;id&gt;</span> on Agents.
        </p>
        <div className="space-y-3">
          {(
            [
              ["global", globalServers, "Global"],
              ["workspace", workspaceServers, "Workspace"],
            ] as const
          ).map(([scope, list, title]) => (
            <FoldCard
              key={scope}
              defaultOpen
              label={<span className="settings-section-title">{title}</span>}
              className="settings-foldcard"
            >
              <div className="space-y-2">
              {list.length === 0 && !(isNew && createScope === scope) ? (
                <p className="px-2 py-3 text-xs text-(--_dk-text-muted)">None.</p>
              ) : null}
              {list.map((server) => (
                <FoldCard
                  key={`${scope}:${server.id}`}
                  open={!isNew && selectedScope === scope && selectedId === server.id}
                  onToggle={(o) => {
                    if (o) {
                      void flushRegisteredSettings().then(() => {
                        setIsNew(false);
                        setSelectedScope(scope);
                        setSelectedId(server.id);
                        setProbe(null);
                      });
                    } else if (selectedScope === scope && selectedId === server.id) {
                      void flushRegisteredSettings().then(() => setSelectedId(null));
                    }
                  }}
                  label={
                    <span className="flex flex-1 items-center justify-between gap-2">
                      <span className="font-mono text-sm text-(--_dk-text-secondary)">
                        {server.id}
                      </span>
                      <span className="flex items-center gap-2">
                        <span className="truncate text-xs text-(--_dk-text-muted)">
                          {server.status ?? "stopped"}
                          {server.command ? ` · ${server.command}` : ""}
                        </span>
                        <button
                          type="button"
                          className="btn-danger btn-icon"
                          disabled={saveBlocked || isPersistBusy(persistStatus)}
                          onClick={(e) => {
                            e.stopPropagation();
                            onDelete(server.id, scope);
                          }}
                          onKeyDown={(e) => e.stopPropagation()}
                          aria-label={`Delete ${server.id}`}
                          title={`Delete ${server.id}`}
                        >
                          <Trash size={16} />
                        </button>
                      </span>
                    </span>
                  }
                  className="settings-foldcard"
                >
                  {!isNew && selectedScope === scope && selectedId === server.id
                    ? editorForm
                    : null}
                </FoldCard>
              ))}
              {isNew && createScope === scope ? (
                <FoldCard
                  key="__new"
                  open
                  onToggle={(o) => {
                    if (!o) setIsNew(false);
                  }}
                  label={
                    <span className="font-mono text-sm text-(--_dk-text-secondary)">(new)</span>
                  }
                  className="settings-foldcard"
                >
                  {editorForm}
                </FoldCard>
              ) : null}
              </div>
            </FoldCard>
          ))}
        </div>
      </div>
    </SettingsPageShell>
  );
}

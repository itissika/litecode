import { useEffect, useState } from "react";
import { Plus, Trash } from "@phosphor-icons/react";

import {
  type McpProbeResult,
  type McpServerItem,
} from "../../../api/settings";
import { useSettingsStore } from "../../../stores/settingsStore";
import { FoldCard } from "../../../components/FoldCard";
import { FieldLabel, TextArea, SettingsPageShell } from "./shared";
import { parseMcpJson } from "./jsonDefinitions";
import {
  flushRegisteredSettings,
  isPersistBusy,
  useSettingsPersist,
} from "./persist";

const EMPTY_MCP_JSON = `{
  "id": "filesystem",
  "command": "npx",
  "args": ["-y", "@modelcontextprotocol/server-filesystem", "/path/to/folder"],
  "env": {},
  "transport": { "type": "stdio" }
}`;

function prettyServer(item: McpServerItem): string {
  return JSON.stringify(
    {
      id: item.id,
      command: item.command,
      args: item.args ?? [],
      env: item.env ?? {},
      transport: item.transport ?? { type: "stdio" },
    },
    null,
    2,
  );
}

export function McpServersSection() {
  const mcpServers = useSettingsStore((s) => s.mcpServers);
  const saving = useSettingsStore((s) => s.saving);
  const saveBlocked = useSettingsStore((s) => s.isSaveBlocked());
  const persistStatus = useSettingsStore((s) => s.persistStatus);
  const setPersistStatus = useSettingsStore((s) => s.setPersistStatus);
  const saveMcpServer = useSettingsStore((s) => s.saveMcpServer);
  const removeMcpServer = useSettingsStore((s) => s.removeMcpServer);
  const startMcp = useSettingsStore((s) => s.startMcpServer);
  const restartMcp = useSettingsStore((s) => s.restartMcpServer);
  const stopMcp = useSettingsStore((s) => s.stopMcpServer);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [jsonText, setJsonText] = useState(EMPTY_MCP_JSON);
  const [formError, setFormError] = useState<string | null>(null);
  const [isNew, setIsNew] = useState(false);
  const [probe, setProbe] = useState<McpProbeResult | null>(null);
  const [busy, setBusy] = useState<"start" | "restart" | "stop" | null>(null);

  const servers = mcpServers ?? [];

  useEffect(() => {
    if (isNew) return;
    if (isPersistBusy(persistStatus)) return;
    if (selectedId) {
      const found = servers.find((s) => s.id === selectedId);
      if (found) {
        setJsonText(prettyServer(found));
        setFormError(null);
        return;
      }
    }
    if (servers.length === 0) {
      setSelectedId(null);
      setJsonText(EMPTY_MCP_JSON);
    }
  }, [servers, selectedId, isNew, persistStatus]);

  useSettingsPersist(jsonText, {
    enabled: !isNew && !!selectedId,
    debounceMs: 400,
    setStatus: setPersistStatus,
    serialize: (text) => parseMcpJson(text, selectedId),
    commit: (p) => saveMcpServer(p.id, p.def),
    revert: () => {
      const found = (useSettingsStore.getState().mcpServers ?? []).find(
        (s) => s.id === selectedId,
      );
      if (found) setJsonText(prettyServer(found));
    },
  });

  const startCreate = () => {
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
        await saveMcpServer(parsed.ok.id, parsed.ok.def);
        setIsNew(false);
        setSelectedId(parsed.ok.id);
        setPersistStatus("saved");
      } catch {
        // toast already shown
      }
    })();
  };

  const onDelete = (id: string) => {
    if (!id || isNew || saveBlocked) return;
    void (async () => {
      try {
        await removeMcpServer(id);
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
    runLifecycle("start", () => startMcp(parsed.ok.id, parsed.ok.def));
  };

  const onRestart = () => {
    const parsed = parseMcpJson(jsonText, isNew ? null : selectedId);
    if ("skip" in parsed) {
      setFormError("Fix JSON before restarting");
      return;
    }
    runLifecycle("restart", () => restartMcp(parsed.ok.id, parsed.ok.def));
  };

  const onStop = () => {
    if (!selectedId) return;
    runLifecycle("stop", () => stopMcp(selectedId));
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
              disabled={saveBlocked || saving}
              onClick={onCreate}
            >
              Create
            </button>
          ) : null}
          <button
            type="button"
            className="btn btn-icon"
            disabled={saveBlocked || saving}
            onClick={startCreate}
            aria-label="New MCP server"
            title="New MCP server"
          >
            <Plus size={16} />
          </button>
        </>
      }
    >
      <div className="space-y-4">
        <p className="text-xs text-(--_dk-text-muted)">
          Register stdio MCP servers as JSON. Start keeps the process; Restart
          applies a new command/env. Then enable{" "}
          <span className="font-mono">mcp_&lt;id&gt;</span> in Tool Catalog and bind on Agents.
        </p>
        {servers.length === 0 && !isNew ? (
          <p className="px-2 py-3 text-xs text-(--_dk-text-muted)">No MCP servers yet.</p>
        ) : null}
        <div className="space-y-2">
          {servers.map((server) => (
            <FoldCard
              key={server.id}
              open={!isNew && selectedId === server.id}
              onToggle={(o) => {
                if (o) {
                  void flushRegisteredSettings().then(() => {
                    setIsNew(false);
                    setSelectedId(server.id);
                    setProbe(null);
                  });
                } else if (selectedId === server.id) {
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
                      disabled={saveBlocked || saving}
                      onClick={(e) => {
                        e.stopPropagation();
                        onDelete(server.id);
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
              {!isNew && selectedId === server.id ? editorForm : null}
            </FoldCard>
          ))}
          {isNew ? (
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
      </div>
    </SettingsPageShell>
  );
}

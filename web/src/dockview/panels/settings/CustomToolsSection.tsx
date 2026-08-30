import { useEffect, useState } from "react";
import { Plus, Trash } from "@phosphor-icons/react";

import {
  type CustomToolDefinition,
  type ToolScope,
} from "../../../api/settings";
import { useSettingsStore } from "../../../stores/settingsStore";
import { FoldCard } from "../../../components/FoldCard";
import { Dropdown, dropdownItemClass } from "../../../components/ui/Dropdown";
import { FieldLabel, TextArea, SettingsPageShell, useSettingsSaveBlocked } from "./shared";
import { parseCustomToolJson } from "./jsonDefinitions";
import {
  flushRegisteredSettings,
  isPersistBusy,
  useSettingsPersist,
} from "./persist";

const EMPTY_CUSTOM_JSON = `{
  "name": "echo_py",
  "description": "Shown to the model when selecting tools",
  "command": "python",
  "args": ["examples/tools/echo_py.py"],
  "timeout": 120,
  "schema": {
    "type": "object",
    "properties": {
      "message": { "type": "string", "description": "Text to echo" }
    },
    "required": ["message"]
  }
}`;

function prettyTool(def: CustomToolDefinition): string {
  return JSON.stringify(
    {
      name: def.name,
      description: def.description ?? "",
      command: def.command,
      args: def.args ?? [],
      timeout: def.timeout ?? 120,
      schema: {
        type: def.schema?.type ?? "object",
        properties: def.schema?.properties ?? {},
        required: def.schema?.required ?? [],
      },
    },
    null,
    2,
  );
}

export function CustomToolsSection() {
  const customTools = useSettingsStore((s) => s.customTools);
  const saveBlocked = useSettingsSaveBlocked();
  const persistStatus = useSettingsStore((s) => s.persistStatus);
  const setPersistStatus = useSettingsStore((s) => s.setPersistStatus);
  const saveCustomTool = useSettingsStore((s) => s.saveCustomTool);
  const removeCustomTool = useSettingsStore((s) => s.removeCustomTool);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [selectedScope, setSelectedScope] = useState<ToolScope>("global");
  const [jsonText, setJsonText] = useState(EMPTY_CUSTOM_JSON);
  const [formError, setFormError] = useState<string | null>(null);
  const [isNew, setIsNew] = useState(false);
  const [createScope, setCreateScope] = useState<ToolScope>("global");

  const globalTools = customTools?.global ?? [];
  const workspaceTools = customTools?.workspace ?? [];
  const tools = selectedScope === "workspace" ? workspaceTools : globalTools;

  useEffect(() => {
    if (isNew) return;
    if (isPersistBusy(persistStatus)) return;
    if (selectedId) {
      const found = tools.find((t) => t.name === selectedId);
      if (found) {
        setJsonText(prettyTool(found));
        setFormError(null);
        return;
      }
    }
    if (tools.length === 0) {
      setSelectedId(null);
      setJsonText(EMPTY_CUSTOM_JSON);
    }
  }, [tools, selectedId, isNew, persistStatus]);

  useSettingsPersist(jsonText, {
    enabled: !isNew && !!selectedId,
    debounceMs: 400,
    setStatus: setPersistStatus,
    serialize: (text) => parseCustomToolJson(text, selectedId),
    commit: (p) => saveCustomTool(p.name, p, selectedScope),
    revert: () => {
      const layered = useSettingsStore.getState().customTools;
      const found = (selectedScope === "workspace"
        ? layered?.workspace
        : layered?.global
      )?.find((t) => t.name === selectedId);
      if (found) setJsonText(prettyTool(found));
    },
  });

  const startCreate = (scope: ToolScope) => {
    setCreateScope(scope);
    setSelectedScope(scope);
    setIsNew(true);
    setSelectedId(null);
    setJsonText(EMPTY_CUSTOM_JSON);
    setFormError(null);
  };

  const onCreate = () => {
    setFormError(null);
    const parsed = parseCustomToolJson(jsonText);
    if ("skip" in parsed) {
      setFormError("Valid JSON with name, command, and schema is required");
      return;
    }
    void (async () => {
      try {
        await saveCustomTool(parsed.ok.name, parsed.ok, createScope);
        setIsNew(false);
        setSelectedScope(createScope);
        setSelectedId(parsed.ok.name);
        setPersistStatus("saved");
      } catch {
        // toast already shown
      }
    })();
  };

  const onDelete = (name: string, scope: ToolScope) => {
    if (!name || isNew || saveBlocked) return;
    void (async () => {
      try {
        await removeCustomTool(name, scope);
        setIsNew(false);
        setSelectedId(null);
      } catch {
        // toast already shown
      }
    })();
  };

  const editorForm = (
    <div className="settings-card space-y-3 p-4">
      <div>
        <FieldLabel required>Definition (JSON)</FieldLabel>
        <TextArea
          rows={18}
          value={jsonText}
          disabled={saveBlocked}
          onChange={(e) => setJsonText(e.target.value)}
          className="font-mono text-xs"
          spellCheck={false}
        />
      </div>
      {formError ? (
        <p className="text-sm text-(--_dk-red-500)">{formError}</p>
      ) : null}
    </div>
  );

  return (
    <SettingsPageShell
      title="Custom Tools"
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
                aria-label="New custom tool"
                aria-haspopup="menu"
                aria-expanded={open}
                title="New custom tool"
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
          Definitions only. Bind on Agents. Same name: workspace overrides global.
        </p>
        <div className="space-y-3">
          {(
            [
              ["global", globalTools, "Global"],
              ["workspace", workspaceTools, "Workspace"],
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
              {list.map((tool) => (
                <FoldCard
                  key={`${scope}:${tool.name}`}
                  open={!isNew && selectedScope === scope && selectedId === tool.name}
                  onToggle={(o) => {
                    if (o) {
                      void flushRegisteredSettings().then(() => {
                        setIsNew(false);
                        setSelectedScope(scope);
                        setSelectedId(tool.name);
                      });
                    } else if (selectedScope === scope && selectedId === tool.name) {
                      void flushRegisteredSettings().then(() => setSelectedId(null));
                    }
                  }}
                  label={
                    <span className="flex flex-1 items-center justify-between gap-2">
                      <span className="font-mono text-sm text-(--_dk-text-secondary)">
                        {tool.name}
                      </span>
                      <span className="flex items-center gap-2">
                        <span className="truncate text-xs text-(--_dk-text-muted)">
                          {tool.command}
                        </span>
                        <button
                          type="button"
                          className="btn-danger btn-icon"
                          disabled={saveBlocked || isPersistBusy(persistStatus)}
                          onClick={(e) => {
                            e.stopPropagation();
                            onDelete(tool.name, scope);
                          }}
                          onKeyDown={(e) => e.stopPropagation()}
                          aria-label={`Delete ${tool.name}`}
                          title={`Delete ${tool.name}`}
                        >
                          <Trash size={16} />
                        </button>
                      </span>
                    </span>
                  }
                  className="settings-foldcard"
                >
                  {!isNew && selectedScope === scope && selectedId === tool.name
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

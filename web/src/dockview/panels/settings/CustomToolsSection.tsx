import { useEffect, useState } from "react";

import {
  type CustomToolDefinition,
  type ToolScope,
} from "../../../api/settings";
import { useSettingsStore } from "../../../stores/settingsStore";
import {
  FieldLabel,
  TextArea,
  useSettingsSaveBlocked,
} from "./shared";
import { parseCustomToolJson } from "./jsonDefinitions";
import { ScopeHint, ScopedDefinitionEditor } from "./ScopedDefinitionEditor";
import {
  flushRegisteredSettings,
  isPersistBusy,
  shouldHydrateDraftFromStore,
  useDocPersist,
  useSettingsPersist,
} from "./persist";

const EMPTY_CUSTOM_JSON = `{
  "name": "echo_py",
  "surface": "direct",
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
  const body: Record<string, unknown> = {
    name: def.name,
    surface: def.surface ?? "direct",
    description: def.description ?? "",
    command: def.command,
    args: def.args ?? [],
    timeout: def.timeout ?? 120,
    schema: {
      type: def.schema?.type ?? "object",
      properties: def.schema?.properties ?? {},
      required: def.schema?.required ?? [],
    },
  };
  if (def.rules && def.rules.length > 0) body.rules = def.rules;
  return JSON.stringify(body, null, 2);
}

export function CustomToolsSection() {
  const customTools = useSettingsStore((s) => s.customTools);
  const saveBlocked = useSettingsSaveBlocked();
  const { persistStatus, setPersistStatus } = useDocPersist("customTools");
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
    if (!shouldHydrateDraftFromStore(persistStatus)) return;
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
      const found = (
        selectedScope === "workspace" ? layered?.workspace : layered?.global
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
        <p className="mb-2 text-xs text-(--_dk-text-muted)">
          Use <code>surface: "direct"</code> for a standalone tool (background and
          media allowed), or <code>surface: "code"</code> for a tool called inside{" "}
          <code>run_code</code>. Code tools return <code>{"{ ok, text }"}</code> and
          cannot use <code>run_in_background</code>.
        </p>
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
    <ScopedDefinitionEditor
      title="Custom Tools"
      newLabel="New custom tool"
      hint={<ScopeHint bindFirst />}
      global={globalTools}
      workspace={workspaceTools}
      itemId={(tool) => tool.name}
      itemSummary={(tool) => `${tool.surface ?? "direct"} · ${tool.command}`}
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

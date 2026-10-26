import { useEffect, useState } from "react";

import {
  type CustomSuiteDefinition,
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
  if (def.suite) body.suite = def.suite;
  else body.surface = def.surface ?? "direct";
  if (def.output_schema) body.output_schema = def.output_schema;
  if (def.rules && def.rules.length > 0) body.rules = def.rules;
  return JSON.stringify(body, null, 2);
}

function prettySuite(def: CustomSuiteDefinition): string {
  const body: Record<string, unknown> = {
    surface: def.surface,
    label: def.label ?? "",
    rules: def.rules ?? [],
  };
  return JSON.stringify(body, null, 2);
}

const EMPTY_SUITE_JSON = `{
  "surface": "code",
  "label": "Deck",
  "rules": []
}`;

function parseSuiteJson(text: string): CustomSuiteDefinition {
  const parsed = JSON.parse(text) as Partial<CustomSuiteDefinition>;
  if (parsed.surface !== "direct" && parsed.surface !== "code") {
    throw new Error('suite needs surface: "direct" or "code"');
  }
  return {
    surface: parsed.surface,
    label: parsed.label ?? "",
    rules: Array.isArray(parsed.rules) ? parsed.rules : [],
  };
}

export function CustomToolsSection() {
  const customTools = useSettingsStore((s) => s.customTools);
  const saveBlocked = useSettingsSaveBlocked();
  const { persistStatus, setPersistStatus } = useDocPersist("customTools");
  const saveCustomTool = useSettingsStore((s) => s.saveCustomTool);
  const removeCustomTool = useSettingsStore((s) => s.removeCustomTool);
  const saveCustomSuite = useSettingsStore((s) => s.saveCustomSuite);
  const removeCustomSuite = useSettingsStore((s) => s.removeCustomSuite);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [selectedScope, setSelectedScope] = useState<ToolScope>("global");
  const [jsonText, setJsonText] = useState(EMPTY_CUSTOM_JSON);
  const [formError, setFormError] = useState<string | null>(null);
  const [isNew, setIsNew] = useState(false);
  const [createScope, setCreateScope] = useState<ToolScope>("global");
  const [suiteScope, setSuiteScope] = useState<ToolScope>("global");
  const [suiteId, setSuiteId] = useState("");
  const [suiteJson, setSuiteJson] = useState(EMPTY_SUITE_JSON);
  const [suiteError, setSuiteError] = useState<string | null>(null);
  const [suiteIsNew, setSuiteIsNew] = useState(false);

  const globalTools = customTools?.global ?? [];
  const workspaceTools = customTools?.workspace ?? [];
  const tools = selectedScope === "workspace" ? workspaceTools : globalTools;
  const suites = customTools?.suites ?? { global: {}, workspace: {} };
  const diagnostics = customTools?.diagnostics ?? [];
  const codeRejections = customTools?.code_rejections ?? [];

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

  const openSuite = (scope: ToolScope, id: string) => {
    const suite = suites[scope][id];
    setSuiteScope(scope);
    setSuiteId(id);
    setSuiteJson(suite ? prettySuite(suite) : EMPTY_SUITE_JSON);
    setSuiteIsNew(false);
    setSuiteError(null);
  };

  const startNewSuite = (scope: ToolScope) => {
    setSuiteScope(scope);
    setSuiteId("");
    setSuiteJson(EMPTY_SUITE_JSON);
    setSuiteIsNew(true);
    setSuiteError(null);
  };

  const onSaveSuite = () => {
    setSuiteError(null);
    const id = suiteId.trim();
    if (!/^[a-z][a-z0-9_]*$/.test(id)) {
      setSuiteError("Suite id must match [a-z][a-z0-9_]*");
      return;
    }
    let parsed: CustomSuiteDefinition;
    try {
      parsed = parseSuiteJson(suiteJson);
    } catch (error) {
      setSuiteError(error instanceof Error ? error.message : "Invalid suite JSON");
      return;
    }
    void (async () => {
      try {
        await saveCustomSuite(id, parsed, suiteScope);
        setSuiteIsNew(false);
        setSuiteError(null);
      } catch (error) {
        setSuiteError(error instanceof Error ? error.message : "Save failed");
      }
    })();
  };

  const onDeleteSuite = () => {
    if (!suiteId || suiteIsNew) return;
    void (async () => {
      try {
        await removeCustomSuite(suiteId, suiteScope);
        setSuiteId("");
        setSuiteJson(EMPTY_SUITE_JSON);
        setSuiteIsNew(false);
        setSuiteError(null);
      } catch (error) {
        setSuiteError(error instanceof Error ? error.message : "Delete failed");
      }
    })();
  };

  const editorForm = (
    <div className="settings-card space-y-3 p-4">
      <div>
        <FieldLabel required>Definition (JSON)</FieldLabel>
        <p className="mb-2 text-xs text-(--_dk-text-muted)">
          Standalone: set <code>surface: "direct"</code> (background and media
          allowed) or <code>surface: "code"</code> and an object{" "}
          <code>output_schema</code>. Suite member: omit <code>surface</code>, set{" "}
          <code>suite</code>, and leave <code>rules</code> empty; the suite owns
          both. Code suite members are called as <code>tools.&lt;suite&gt;.&lt;member&gt;</code>.
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

  const suiteScopes: ToolScope[] = ["global", "workspace"];
  const suitesView = (
    <div className="settings-card space-y-3 p-4">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div>
          <FieldLabel>Suites</FieldLabel>
          <p className="text-xs text-(--_dk-text-muted)">
            One suite is one card, one binding, and one permission face. A
            workspace suite replaces the global suite and all of its members.
          </p>
        </div>
        <div className="flex gap-2">
          <button
            type="button"
            className="btn btn-sm"
            disabled={saveBlocked}
            onClick={() => startNewSuite("global")}
          >
            New global
          </button>
          <button
            type="button"
            className="btn btn-sm"
            disabled={saveBlocked}
            onClick={() => startNewSuite("workspace")}
          >
            New workspace
          </button>
        </div>
      </div>
      {suiteScopes.map((scope) => {
        const entries = Object.entries(suites[scope]).sort(([a], [b]) =>
          a.localeCompare(b),
        );
        return (
          <div key={scope} className="space-y-1">
            <p className="text-xs font-medium text-(--_dk-text-muted)">
              {scope === "global" ? "Global" : "Workspace"}
            </p>
            {entries.length === 0 ? (
              <p className="text-xs text-(--_dk-text-muted)">None.</p>
            ) : (
              entries.map(([id, suite]) => (
                <button
                  key={id}
                  type="button"
                  className="block w-full rounded px-2 py-1 text-left text-xs hover:bg-(--_dk-bg-hover)"
                  onClick={() => openSuite(scope, id)}
                >
                  <span className="font-mono">{id}</span> · {suite.surface} ·{" "}
                  {suite.label || "no label"} · {(suite.rules ?? []).length} rule(s)
                </button>
              ))
            )}
          </div>
        );
      })}
      {suiteIsNew || suiteId ? (
        <div className="space-y-2 border-t border-(--_dk-border) pt-3">
          <div className="flex flex-wrap items-center gap-2">
            <input
              className="input input-sm font-mono"
              value={suiteId}
              placeholder="suite_id"
              disabled={saveBlocked || !suiteIsNew}
              onChange={(event) => setSuiteId(event.target.value)}
            />
            <span className="text-xs text-(--_dk-text-muted)">
              {suiteScope === "global" ? "global" : "workspace"}
            </span>
            <button
              type="button"
              className="btn-primary btn-sm"
              disabled={saveBlocked}
              onClick={onSaveSuite}
            >
              Save suite
            </button>
            {!suiteIsNew ? (
              <button
                type="button"
                className="btn btn-sm"
                disabled={saveBlocked}
                onClick={onDeleteSuite}
              >
                Delete suite
              </button>
            ) : null}
            <button
              type="button"
              className="btn btn-sm"
              onClick={() => {
                setSuiteId("");
                setSuiteIsNew(false);
                setSuiteError(null);
              }}
            >
              Close
            </button>
          </div>
          <TextArea
            rows={8}
            value={suiteJson}
            disabled={saveBlocked}
            onChange={(event) => setSuiteJson(event.target.value)}
            className="font-mono text-xs"
            spellCheck={false}
          />
          {suiteError ? (
            <p className="text-sm text-(--_dk-red-500)">{suiteError}</p>
          ) : null}
        </div>
      ) : null}
    </div>
  );

  const codeRejectionsView =
    codeRejections.length > 0 ? (
      <div className="settings-card space-y-1 p-4">
        <FieldLabel>Code admission warnings</FieldLabel>
        <p className="text-xs text-(--_dk-text-muted)">
          These tools cannot enter <code>run_code</code>; direct definitions are
          not affected.
        </p>
        {codeRejections.map((rejection, index) => (
          <p
            key={`code:${rejection.name}:${index}`}
            className="text-xs text-(--_dk-amber-500)"
          >
            <span className="font-mono">{rejection.name}</span>: {rejection.reason}
          </p>
        ))}
      </div>
    ) : null;

  const diagnosticsView =
    diagnostics.length > 0 ? (
      <div className="settings-card space-y-1 p-4">
        <FieldLabel>Definition errors</FieldLabel>
        {diagnostics.map((diagnostic, index) => (
          <p
            key={`${diagnostic.source}:${diagnostic.tool ?? diagnostic.suite ?? ""}:${diagnostic.field}:${index}`}
            className="text-xs text-(--_dk-red-500)"
          >
            {diagnostic.message}
            <span className="text-(--_dk-text-muted)"> Fix: {diagnostic.fix}</span>
          </p>
        ))}
      </div>
    ) : null;

  return (
    <ScopedDefinitionEditor
      title="Custom Tools"
      newLabel="New custom tool"
      hint={
        <div className="space-y-3">
          <ScopeHint bindFirst />
          <p className="text-xs text-(--_dk-text-muted)">
            Invalid definitions stay visible below and never enter the catalog.
            Old formats are not migrated: rewrite them in the direct/code + suites
            format shown in the guide.
          </p>
          {codeRejectionsView}
          {diagnosticsView}
          {suitesView}
        </div>
      }
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

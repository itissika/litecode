import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Plus, Trash } from "@phosphor-icons/react";

import {
  type CustomSuiteDefinition,
  type CustomToolDefinition,
  type ToolScope,
} from "../../../api/settings";
import { FoldCard } from "../../../components/FoldCard";
import { Dropdown, dropdownItemClass } from "../../../components/ui/Dropdown";
import i18n from "../../../i18n";
import { useSettingsStore } from "../../../stores/settingsStore";
import { parseCustomToolJson } from "./jsonDefinitions";
import {
  flushRegisteredSettings,
  isPersistBusy,
  shouldHydrateDraftFromStore,
  useDocPersist,
  useSettingsPersist,
} from "./persist";
import {
  FieldLabel,
  SettingsPageShell,
  TextArea,
  useSettingsSaveBlocked,
} from "./shared";

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
    throw new Error(i18n.t("settings.customTools.suiteSurfaceRequired"));
  }
  return {
    surface: parsed.surface,
    label: parsed.label ?? "",
    rules: Array.isArray(parsed.rules) ? parsed.rules : [],
  };
}

const SCOPES: readonly ToolScope[] = ["global", "workspace"];

type SuitePayload = { id: string; def: CustomSuiteDefinition };

type Selection =
  | { kind: "tool"; scope: ToolScope; id: string }
  | { kind: "new-tool"; scope: ToolScope }
  | { kind: "suite"; scope: ToolScope; id: string }
  | { kind: "new-suite"; scope: ToolScope };

export function CustomToolsSection() {
  const { t } = useTranslation();
  const customTools = useSettingsStore((s) => s.customTools);
  const saveBlocked = useSettingsSaveBlocked();
  const { persistStatus, setPersistStatus } = useDocPersist("customTools");
  const saveCustomTool = useSettingsStore((s) => s.saveCustomTool);
  const removeCustomTool = useSettingsStore((s) => s.removeCustomTool);
  const saveCustomSuite = useSettingsStore((s) => s.saveCustomSuite);
  const removeCustomSuite = useSettingsStore((s) => s.removeCustomSuite);

  const [selection, setSelection] = useState<Selection | null>(null);
  const [toolDraft, setToolDraft] = useState(EMPTY_CUSTOM_JSON);
  const [toolError, setToolError] = useState<string | null>(null);
  const [suiteDraftId, setSuiteDraftId] = useState("");
  const [suiteDraft, setSuiteDraft] = useState(EMPTY_SUITE_JSON);
  const [suiteError, setSuiteError] = useState<string | null>(null);

  const globalTools = customTools?.global ?? [];
  const workspaceTools = customTools?.workspace ?? [];
  const suites = customTools?.suites ?? { global: {}, workspace: {} };
  const diagnostics = customTools?.diagnostics ?? [];
  const codeRejections = customTools?.code_rejections ?? [];

  const editingTool = selection?.kind === "tool" ? selection : null;
  const editingToolScope = editingTool?.scope ?? null;
  const editingToolId = editingTool?.id ?? null;
  const newToolScope = selection?.kind === "new-tool" ? selection.scope : null;
  const editingSuite = selection?.kind === "suite" ? selection : null;
  const editingSuiteScope = editingSuite?.scope ?? null;
  const editingSuiteId = editingSuite?.id ?? null;
  const newSuiteScope = selection?.kind === "new-suite" ? selection.scope : null;
  const actionsDisabled = saveBlocked || isPersistBusy(persistStatus);

  useEffect(() => {
    if (!editingToolScope || !editingToolId) return;
    if (!shouldHydrateDraftFromStore(persistStatus)) return;
    const list =
      editingToolScope === "workspace" ? workspaceTools : globalTools;
    const found = list.find((tool) => tool.name === editingToolId);
    if (!found) return;
    setToolDraft(prettyTool(found));
    setToolError(null);
  }, [
    editingToolScope,
    editingToolId,
    globalTools,
    workspaceTools,
    persistStatus,
  ]);

  useSettingsPersist(toolDraft, {
    enabled: editingTool !== null,
    debounceMs: 400,
    setStatus: setPersistStatus,
    serialize: (text) => parseCustomToolJson(text, editingToolId),
    commit: (payload) => {
      if (!editingTool) return Promise.resolve();
      return saveCustomTool(payload.name, payload, editingTool.scope);
    },
    revert: () => {
      if (!editingToolScope || !editingToolId) return;
      const layered = useSettingsStore.getState().customTools;
      const list =
        editingToolScope === "workspace" ? layered?.workspace : layered?.global;
      const found = list?.find((tool) => tool.name === editingToolId);
      if (found) setToolDraft(prettyTool(found));
    },
  });

  useEffect(() => {
    if (!editingSuiteScope || !editingSuiteId) return;
    if (!shouldHydrateDraftFromStore(persistStatus)) return;
    const suite = suites[editingSuiteScope]?.[editingSuiteId];
    if (!suite) return;
    setSuiteDraft(prettySuite(suite));
  }, [editingSuiteScope, editingSuiteId, suites, persistStatus]);

  useSettingsPersist<string, SuitePayload>(suiteDraft, {
    enabled: editingSuite !== null,
    debounceMs: 400,
    setStatus: setPersistStatus,
    serialize: (text) => {
      if (!editingSuite) return { skip: "invalid" };
      try {
        return { ok: { id: editingSuite.id, def: parseSuiteJson(text) } };
      } catch {
        return { skip: "invalid" };
      }
    },
    commit: (payload) => {
      if (!editingSuite) return Promise.resolve();
      return saveCustomSuite(payload.id, payload.def, editingSuite.scope);
    },
    revert: () => {
      if (!editingSuite) return;
      const layered = useSettingsStore.getState().customTools;
      const suite =
        layered?.suites?.[editingSuite.scope]?.[editingSuite.id];
      if (suite) setSuiteDraft(prettySuite(suite));
    },
  });

  const closeEditor = () => {
    void flushRegisteredSettings().then(() => setSelection(null));
  };

  const openTool = (scope: ToolScope, id: string) => {
    void flushRegisteredSettings().then(() => {
      const layered = useSettingsStore.getState().customTools;
      const list = scope === "workspace" ? layered?.workspace : layered?.global;
      const found = list?.find((tool) => tool.name === id);
      if (!found) return;
      setSelection({ kind: "tool", scope, id });
      setToolDraft(prettyTool(found));
      setToolError(null);
    });
  };

  const startNewTool = (scope: ToolScope) => {
    void flushRegisteredSettings().then(() => {
      setSelection({ kind: "new-tool", scope });
      setToolDraft(EMPTY_CUSTOM_JSON);
      setToolError(null);
    });
  };

  const onCreate = () => {
    if (selection?.kind !== "new-tool") return;
    const scope = selection.scope;
    setToolError(null);
    const parsed = parseCustomToolJson(toolDraft);
    if ("skip" in parsed) {
      setToolError(t("settings.customTools.jsonRequired"));
      return;
    }
    void (async () => {
      try {
        await saveCustomTool(parsed.ok.name, parsed.ok, scope);
        setSelection({ kind: "tool", scope, id: parsed.ok.name });
        setPersistStatus("saved");
      } catch {
        // toast already shown
      }
    })();
  };

  const deleteTool = (scope: ToolScope, id: string) => {
    if (actionsDisabled) return;
    void (async () => {
      try {
        await removeCustomTool(id, scope);
        setSelection((current) =>
          current?.kind === "tool" &&
          current.scope === scope &&
          current.id === id
            ? null
            : current,
        );
      } catch {
        // toast already shown
      }
    })();
  };

  const openSuite = (scope: ToolScope, id: string) => {
    void flushRegisteredSettings().then(() => {
      const layered = useSettingsStore.getState().customTools;
      const suite = layered?.suites?.[scope]?.[id];
      setSelection({ kind: "suite", scope, id });
      setSuiteDraftId(id);
      setSuiteDraft(suite ? prettySuite(suite) : EMPTY_SUITE_JSON);
      setSuiteError(null);
    });
  };

  const startNewSuite = (scope: ToolScope) => {
    void flushRegisteredSettings().then(() => {
      setSelection({ kind: "new-suite", scope });
      setSuiteDraftId("");
      setSuiteDraft(EMPTY_SUITE_JSON);
      setSuiteError(null);
    });
  };

  const onSaveSuite = () => {
    if (selection?.kind !== "suite" && selection?.kind !== "new-suite") return;
    const scope = selection.scope;
    setSuiteError(null);
    const id = suiteDraftId.trim();
    if (!/^[a-z][a-z0-9_]*$/.test(id)) {
      setSuiteError(t("settings.customTools.suiteIdInvalid"));
      return;
    }
    let parsed: CustomSuiteDefinition;
    try {
      parsed = parseSuiteJson(suiteDraft);
    } catch (error) {
      setSuiteError(
        error instanceof Error
          ? error.message
          : t("settings.customTools.invalidSuiteJson"),
      );
      return;
    }
    void (async () => {
      try {
        await saveCustomSuite(id, parsed, scope);
        setSelection({ kind: "suite", scope, id });
        setSuiteDraftId(id);
      } catch (error) {
        setSuiteError(
          error instanceof Error
            ? error.message
            : t("settings.customTools.saveFailed"),
        );
      }
    })();
  };

  const deleteSuite = (scope: ToolScope, id: string) => {
    if (actionsDisabled) return;
    void (async () => {
      try {
        await removeCustomSuite(id, scope);
        setSelection((current) =>
          current?.kind === "suite" &&
          current.scope === scope &&
          current.id === id
            ? null
            : current,
        );
      } catch {
        // toast already shown
      }
    })();
  };

  const toolSummary = (tool: CustomToolDefinition) =>
    tool.suite
      ? `${t("settings.customTools.memberOf", { suite: tool.suite })} · ${tool.command}`
      : `${tool.surface ?? "direct"} · ${tool.command}`;

  const suiteSummary = (id: string, suite: CustomSuiteDefinition) => {
    const parts = [t("settings.customTools.suiteTag"), suite.surface];
    if (suite.label && suite.label !== id) parts.push(suite.label);
    const rules = suite.rules?.length ?? 0;
    if (rules > 0) {
      parts.push(t("settings.customTools.rulesCount", { count: rules }));
    }
    return parts.join(" · ");
  };

  const toolEditor = (isNew: boolean) => (
    <div className="settings-card space-y-3 p-4">
      <TextArea
        rows={16}
        value={toolDraft}
        disabled={saveBlocked}
        onChange={(event) => setToolDraft(event.target.value)}
        className="font-mono text-xs"
        spellCheck={false}
        aria-label={t("settings.mcp.definitionJson")}
      />
      {toolError ? (
        <p className="text-sm text-(--_dk-red-500)">{toolError}</p>
      ) : null}
      {isNew ? (
        <div className="flex flex-wrap items-center gap-2">
          <button
            type="button"
            className="btn-ghost"
            disabled={saveBlocked}
            onClick={() => setSelection(null)}
          >
            {t("common.cancel")}
          </button>
        </div>
      ) : null}
    </div>
  );

  const suiteEditor = (isNew: boolean) => (
    <div className="settings-card space-y-3 p-4">
      {isNew ? (
        <input
          className="settings-input max-w-56 font-mono text-xs"
          value={suiteDraftId}
          placeholder={t("settings.customTools.suiteIdPlaceholder")}
          disabled={saveBlocked}
          onChange={(event) => setSuiteDraftId(event.target.value)}
          aria-label={t("settings.customTools.suiteIdPlaceholder")}
        />
      ) : null}
      <TextArea
        rows={10}
        value={suiteDraft}
        disabled={saveBlocked}
        onChange={(event) => setSuiteDraft(event.target.value)}
        className="font-mono text-xs"
        spellCheck={false}
        aria-label={t("settings.mcp.definitionJson")}
      />
      {suiteError ? (
        <p className="text-sm text-(--_dk-red-500)">{suiteError}</p>
      ) : null}
      {isNew ? (
        <div className="flex flex-wrap items-center gap-2">
          <button
            type="button"
            className="btn-ghost"
            disabled={saveBlocked}
            onClick={() => setSelection(null)}
          >
            {t("common.cancel")}
          </button>
        </div>
      ) : null}
    </div>
  );

  const codeRejectionsView =
    codeRejections.length > 0 ? (
      <div className="settings-card space-y-1 p-4">
        <FieldLabel>{t("settings.customTools.codeWarnings")}</FieldLabel>
        {codeRejections.map((rejection, index) => (
          <p
            key={`code:${rejection.name}:${index}`}
            className="text-xs text-(--_dk-amber-500)"
          >
            <span className="font-mono">{rejection.name}</span>:{" "}
            {rejection.reason}
          </p>
        ))}
      </div>
    ) : null;

  const diagnosticsView =
    diagnostics.length > 0 ? (
      <div className="settings-card space-y-1 p-4">
        <FieldLabel>{t("settings.customTools.definitionErrors")}</FieldLabel>
        {diagnostics.map((diagnostic, index) => (
          <p
            key={`${diagnostic.source}:${diagnostic.tool ?? diagnostic.suite ?? ""}:${diagnostic.field}:${index}`}
            className="text-xs text-(--_dk-red-500)"
          >
            {diagnostic.message}
            <span className="text-(--_dk-text-muted)">
              {" "}
              {t("common.fix", { fix: diagnostic.fix })}
            </span>
          </p>
        ))}
      </div>
    ) : null;

  const addMenu = (
    <Dropdown
      variant="menu"
      align="right"
      panelClassName="min-w-[190px] rounded-md py-1"
      trigger={({ open, toggle }) => (
        <button
          type="button"
          className="btn btn-icon"
          disabled={actionsDisabled}
          aria-label={t("common.new")}
          aria-haspopup="menu"
          aria-expanded={open}
          title={t("common.new")}
          onClick={toggle}
        >
          <Plus size={16} />
        </button>
      )}
    >
      <button
        type="button"
        className={dropdownItemClass}
        onClick={() => startNewTool("global")}
      >
        {t("settings.customTools.newGlobalTool")}
      </button>
      <button
        type="button"
        className={dropdownItemClass}
        onClick={() => startNewTool("workspace")}
      >
        {t("settings.customTools.newWorkspaceTool")}
      </button>
      <div className="my-1 border-t border-(--_dk-line)" />
      <button
        type="button"
        className={dropdownItemClass}
        onClick={() => startNewSuite("global")}
      >
        {t("settings.customTools.newGlobalSuite")}
      </button>
      <button
        type="button"
        className={dropdownItemClass}
        onClick={() => startNewSuite("workspace")}
      >
        {t("settings.customTools.newWorkspaceSuite")}
      </button>
    </Dropdown>
  );

  return (
    <SettingsPageShell
      title={t("settings.customTools.title")}
      actions={
        <>
          {selection?.kind === "new-tool" ? (
            <button
              type="button"
              className="btn-primary btn-sm"
              disabled={actionsDisabled}
              onClick={onCreate}
            >
              {t("common.create")}
            </button>
          ) : null}
          {selection?.kind === "new-suite" ? (
            <button
              type="button"
              className="btn-primary btn-sm"
              disabled={actionsDisabled}
              onClick={onSaveSuite}
            >
              {t("settings.customTools.saveSuite")}
            </button>
          ) : null}
          {addMenu}
        </>
      }
    >
      <div className="space-y-3">
        {codeRejectionsView}
        {diagnosticsView}
        {SCOPES.map((scope) => {
          const scopeTools = [
            ...(scope === "workspace" ? workspaceTools : globalTools),
          ].sort((a, b) => a.name.localeCompare(b.name));
          const scopeSuites = Object.entries(suites[scope]).sort(([a], [b]) =>
            a.localeCompare(b),
          );
          const newToolHere = newToolScope === scope;
          const newSuiteHere = newSuiteScope === scope;
          return (
            <FoldCard
              key={scope}
              defaultOpen
              label={
                <span className="settings-section-title">
                  {scope === "global"
                    ? t("common.global")
                    : t("common.workspace")}
                </span>
              }
              className="settings-foldcard"
            >
              <div className="space-y-2">
                {scopeSuites.length === 0 &&
                scopeTools.length === 0 &&
                !newToolHere &&
                !newSuiteHere ? (
                  <p className="px-2 py-1 text-xs text-(--_dk-text-muted)">
                    {t("common.none")}
                  </p>
                ) : null}
                {scopeSuites.map(([id, suite]) => {
                  const isOpen =
                    editingSuite?.scope === scope && editingSuite.id === id;
                  return (
                    <FoldCard
                      key={`suite:${id}`}
                      open={isOpen}
                      onToggle={(open) => {
                        if (open) openSuite(scope, id);
                        else if (isOpen) closeEditor();
                      }}
                      label={
                        <span className="flex min-w-0 flex-1 items-center justify-between gap-2">
                          <span className="truncate font-mono text-sm text-(--_dk-text-secondary)">
                            {id}
                          </span>
                          <span className="flex items-center gap-2">
                            <span className="truncate text-xs text-(--_dk-text-muted)">
                              {suiteSummary(id, suite)}
                            </span>
                            <button
                              type="button"
                              className="btn-danger btn-icon"
                              disabled={actionsDisabled}
                              onClick={(event) => {
                                event.stopPropagation();
                                deleteSuite(scope, id);
                              }}
                              onKeyDown={(event) => event.stopPropagation()}
                              aria-label={t("common.deleteNamed", { id })}
                              title={t("common.deleteNamed", { id })}
                            >
                              <Trash size={16} />
                            </button>
                          </span>
                        </span>
                      }
                      className="settings-foldcard"
                    >
                      {isOpen ? suiteEditor(false) : null}
                    </FoldCard>
                  );
                })}
                {scopeTools.map((tool) => {
                  const isOpen =
                    editingTool?.scope === scope && editingTool.id === tool.name;
                  return (
                    <FoldCard
                      key={`tool:${tool.name}`}
                      open={isOpen}
                      onToggle={(open) => {
                        if (open) openTool(scope, tool.name);
                        else if (isOpen) closeEditor();
                      }}
                      label={
                        <span className="flex min-w-0 flex-1 items-center justify-between gap-2">
                          <span className="truncate font-mono text-sm text-(--_dk-text-secondary)">
                            {tool.name}
                          </span>
                          <span className="flex items-center gap-2">
                            <span className="truncate text-xs text-(--_dk-text-muted)">
                              {toolSummary(tool)}
                            </span>
                            <button
                              type="button"
                              className="btn-danger btn-icon"
                              disabled={actionsDisabled}
                              onClick={(event) => {
                                event.stopPropagation();
                                deleteTool(scope, tool.name);
                              }}
                              onKeyDown={(event) => event.stopPropagation()}
                              aria-label={t("common.deleteNamed", {
                                id: tool.name,
                              })}
                              title={t("common.deleteNamed", {
                                id: tool.name,
                              })}
                            >
                              <Trash size={16} />
                            </button>
                          </span>
                        </span>
                      }
                      className="settings-foldcard"
                    >
                      {isOpen ? toolEditor(false) : null}
                    </FoldCard>
                  );
                })}
                {newSuiteHere ? (
                  <FoldCard
                    open
                    onToggle={(open) => {
                      if (!open) setSelection(null);
                    }}
                    label={
                      <span className="font-mono text-sm text-(--_dk-text-secondary)">
                        {t("settings.customTools.newSuite")}
                      </span>
                    }
                    className="settings-foldcard"
                  >
                    {suiteEditor(true)}
                  </FoldCard>
                ) : null}
                {newToolHere ? (
                  <FoldCard
                    open
                    onToggle={(open) => {
                      if (!open) setSelection(null);
                    }}
                    label={
                      <span className="font-mono text-sm text-(--_dk-text-secondary)">
                        {t("settings.customTools.newTool")}
                      </span>
                    }
                    className="settings-foldcard"
                  >
                    {toolEditor(true)}
                  </FoldCard>
                ) : null}
              </div>
            </FoldCard>
          );
        })}
      </div>
    </SettingsPageShell>
  );
}

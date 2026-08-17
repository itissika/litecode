import { useEffect, useMemo, useState } from "react";
import { Plus, Trash } from "@phosphor-icons/react";

import { type CustomToolDefinition } from "../../../api/settings";
import { useSettingsStore } from "../../../stores/settingsStore";
import { FoldCard } from "../../../components/FoldCard";
import {
  FieldLabel,
  TextInput,
  TextArea,
  SettingsPageShell,
} from "./shared";
import {
  flushRegisteredSettings,
  isPersistBusy,
  useSettingsPersist,
  type SerializeResult,
} from "./persist";

const EMPTY_CUSTOM_TOOL: CustomToolDefinition = {
  name: "",
  description: "",
  schema: { type: "object", properties: {}, required: [] },
  command: "",
  args: [],
  timeout: 120,
};

type CustomToolDraft = {
  draft: CustomToolDefinition;
  propertiesText: string;
  requiredText: string;
  argsText: string;
};

function parseCustomTool(
  state: CustomToolDraft,
): SerializeResult<CustomToolDefinition> {
  const name = state.draft.name.trim();
  if (!/^[a-z][a-z0-9_]*$/.test(name)) return { skip: "invalid" };
  if (!state.draft.command.trim()) return { skip: "invalid" };
  try {
    const properties = JSON.parse(state.propertiesText) as Record<string, unknown>;
    if (properties === null || typeof properties !== "object" || Array.isArray(properties)) {
      return { skip: "invalid" };
    }
    const parsed = JSON.parse(state.requiredText) as unknown;
    if (!Array.isArray(parsed) || parsed.some((x) => typeof x !== "string")) {
      return { skip: "invalid" };
    }
    const args = state.argsText
      .split("\n")
      .map((line) => line.trim())
      .filter(Boolean);
    return {
      ok: {
        name,
        description: state.draft.description?.trim() ?? "",
        command: state.draft.command.trim(),
        args,
        timeout: state.draft.timeout && state.draft.timeout > 0 ? state.draft.timeout : 120,
        schema: {
          type: "object",
          properties,
          required: parsed as string[],
        },
      },
    };
  } catch {
    return { skip: "invalid" };
  }
}

function applyToolSnapshot(found: CustomToolDefinition): CustomToolDraft {
  return {
    draft: {
      ...found,
      description: found.description ?? "",
      args: found.args ?? [],
      timeout: found.timeout ?? 120,
    },
    propertiesText: JSON.stringify(found.schema?.properties ?? {}, null, 2),
    requiredText: JSON.stringify(found.schema?.required ?? [], null, 2),
    argsText: (found.args ?? []).join("\n"),
  };
}

export function CustomToolsSection() {
  const customTools = useSettingsStore((s) => s.customTools);
  const saving = useSettingsStore((s) => s.saving);
  const saveBlocked = useSettingsStore((s) => s.isSaveBlocked());
  const persistStatus = useSettingsStore((s) => s.persistStatus);
  const setPersistStatus = useSettingsStore((s) => s.setPersistStatus);
  const saveCustomTool = useSettingsStore((s) => s.saveCustomTool);
  const removeCustomTool = useSettingsStore((s) => s.removeCustomTool);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [draft, setDraft] = useState<CustomToolDefinition>(EMPTY_CUSTOM_TOOL);
  const [propertiesText, setPropertiesText] = useState("{}");
  const [requiredText, setRequiredText] = useState("[]");
  const [argsText, setArgsText] = useState("");
  const [formError, setFormError] = useState<string | null>(null);
  const [isNew, setIsNew] = useState(false);

  const tools = customTools ?? [];
  const persistDraft: CustomToolDraft = useMemo(
    () => ({ draft, propertiesText, requiredText, argsText }),
    [draft, propertiesText, requiredText, argsText],
  );

  useEffect(() => {
    if (isNew) return;
    if (isPersistBusy(persistStatus)) return;
    if (selectedId) {
      const found = tools.find((t) => t.name === selectedId);
      if (found) {
        const snap = applyToolSnapshot(found);
        setDraft(snap.draft);
        setPropertiesText(snap.propertiesText);
        setRequiredText(snap.requiredText);
        setArgsText(snap.argsText);
        setFormError(null);
        return;
      }
    }
    if (tools.length === 0) {
      setSelectedId(null);
      setDraft(EMPTY_CUSTOM_TOOL);
      setPropertiesText("{}");
      setRequiredText("[]");
      setArgsText("");
    }
  }, [tools, selectedId, isNew, persistStatus]);

  useSettingsPersist(persistDraft, {
    enabled: !isNew && !!selectedId,
    debounceMs: 400,
    setStatus: setPersistStatus,
    serialize: parseCustomTool,
    commit: (p) => saveCustomTool(p.name, p),
    revert: () => {
      const id = useSettingsStore.getState();
      const name = selectedId;
      const found = (id.customTools ?? []).find((t) => t.name === name);
      if (!found) return;
      const snap = applyToolSnapshot(found);
      setDraft(snap.draft);
      setPropertiesText(snap.propertiesText);
      setRequiredText(snap.requiredText);
      setArgsText(snap.argsText);
    },
  });

  const startCreate = () => {
    setIsNew(true);
    setSelectedId(null);
    setDraft({ ...EMPTY_CUSTOM_TOOL });
    setPropertiesText(
      JSON.stringify({ message: { type: "string", description: "Text to echo" } }, null, 2),
    );
    setRequiredText(JSON.stringify(["message"], null, 2));
    setArgsText("");
    setFormError(null);
  };

  const onCreate = () => {
    setFormError(null);
    const parsed = parseCustomTool(persistDraft);
    if ("skip" in parsed) {
      setFormError("Name, command, and valid schema JSON are required");
      return;
    }
    void (async () => {
      try {
        await saveCustomTool(parsed.ok.name, parsed.ok);
        setIsNew(false);
        setSelectedId(parsed.ok.name);
        setPersistStatus("saved");
      } catch {
        // toast already shown
      }
    })();
  };

  const onDelete = (name: string) => {
    if (!name || isNew || saveBlocked) return;
    void (async () => {
      try {
        await removeCustomTool(name);
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
        <FieldLabel required>Name</FieldLabel>
        <TextInput
          value={draft.name}
          disabled={saveBlocked || (!isNew && !!selectedId)}
          onChange={(e) => setDraft((d) => ({ ...d, name: e.target.value }))}
          placeholder="echo_py"
        />
      </div>
      <div>
        <FieldLabel>Description</FieldLabel>
        <TextArea
          rows={2}
          value={draft.description ?? ""}
          disabled={saveBlocked}
          onChange={(e) => setDraft((d) => ({ ...d, description: e.target.value }))}
          placeholder="Shown to the model when selecting tools"
        />
      </div>
      <div>
        <FieldLabel required>Command</FieldLabel>
        <TextInput
          value={draft.command}
          disabled={saveBlocked}
          onChange={(e) => setDraft((d) => ({ ...d, command: e.target.value }))}
          placeholder="python"
        />
      </div>
      <div>
        <FieldLabel>Args (one per line)</FieldLabel>
        <TextArea
          rows={3}
          value={argsText}
          disabled={saveBlocked}
          onChange={(e) => setArgsText(e.target.value)}
          placeholder={"examples/tools/echo_py.py"}
          className="font-mono text-xs"
        />
      </div>
      <div>
        <FieldLabel>Timeout (seconds)</FieldLabel>
        <TextInput
          type="number"
          min={1}
          value={draft.timeout ?? 120}
          disabled={saveBlocked}
          onChange={(e) =>
            setDraft((d) => ({ ...d, timeout: Number(e.target.value) || 120 }))
          }
        />
      </div>
      <div>
        <FieldLabel required>Schema properties (JSON)</FieldLabel>
        <TextArea
          rows={6}
          value={propertiesText}
          disabled={saveBlocked}
          onChange={(e) => setPropertiesText(e.target.value)}
          className="font-mono text-xs"
        />
      </div>
      <div>
        <FieldLabel>Schema required (JSON array)</FieldLabel>
        <TextArea
          rows={2}
          value={requiredText}
          disabled={saveBlocked}
          onChange={(e) => setRequiredText(e.target.value)}
          className="font-mono text-xs"
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
            aria-label="New custom tool"
            title="New custom tool"
          >
            <Plus size={16} />
          </button>
        </>
      }
    >
      <div className="space-y-4">
      <p className="text-xs text-(--_dk-text-muted)">
        Definitions are stored in the global DB. After save: enable in Tool Catalog, then bind on Agents.
        Protocol: stdin JSON → stdout result; exit 0 success, 2 blocked.
      </p>
      {tools.length === 0 && !isNew ? (
        <p className="px-2 py-3 text-xs text-(--_dk-text-muted)">No custom tools yet.</p>
      ) : null}
      <div className="space-y-2">
        {tools.map((tool) => (
          <FoldCard
            key={tool.name}
            open={!isNew && selectedId === tool.name}
            onToggle={(o) => {
              if (o) {
                void flushRegisteredSettings().then(() => {
                  setIsNew(false);
                  setSelectedId(tool.name);
                });
              } else if (selectedId === tool.name) {
                void flushRegisteredSettings().then(() => setSelectedId(null));
              }
            }}
            label={
              <span className="flex flex-1 items-center justify-between gap-2">
                <span className="font-mono text-sm text-(--_dk-text-secondary)">
                  {tool.name}
                </span>
                <span className="flex items-center gap-2">
                  <span className="text-xs text-(--_dk-text-muted) truncate">
                    {tool.command}
                  </span>
                  <button
                    type="button"
                    className="btn-danger btn-icon"
                    disabled={saveBlocked || saving}
                    onClick={(e) => {
                      e.stopPropagation();
                      onDelete(tool.name);
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
            {!isNew && selectedId === tool.name ? editorForm : null}
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

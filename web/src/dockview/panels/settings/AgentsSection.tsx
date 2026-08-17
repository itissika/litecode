import { useEffect, useMemo, useState, type CSSProperties, type ReactNode } from "react";
import { Plus, ArrowClockwise } from "@phosphor-icons/react";

import {
  isConfigurableTool,
  isAgentBindableTool,
  isHiddenSettingsAgent,
  isProtectedAgent,
  isSubagentBindableTool,
  modelOptionLabel,
  applyToolEnabled,
  withSyncedToolSeries,
  type AgentProfile,
  type AgentToolBinding,
  type ModelDefinition,
  type ToolCatalogEntry,
} from "../../../api/settings";
import { useSettingsStore } from "../../../stores/settingsStore";
import { Select } from "../../../components/ui/Select";
import { AgentTypeIcon, agentColor } from "../../../components/agentIdentity";
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
} from "./persist";

function ModelRefSelect({
  value,
  models,
  onChange,
  disabled,
}: {
  value: string;
  models: Record<string, ModelDefinition> | null;
  onChange: (modelRef: string) => void;
  disabled?: boolean;
}) {
  const options = useMemo(
    () => {
      const entries = Object.values(models ?? {}).sort((a, b) =>
        modelOptionLabel(a).localeCompare(modelOptionLabel(b)),
      );
      return [
        { value: "", label: "— select —" as ReactNode },
        ...entries.map((m) => ({ value: m.id, label: modelOptionLabel(m) as ReactNode })),
      ];
    },
    [models],
  );

  const locked = disabled || options.length <= 1;

  return (
    <Select
      value={value}
      onChange={onChange}
      options={options}
      disabled={locked}
      className="w-full"
    />
  );
}

function bindingFor(
  tools: Record<string, AgentToolBinding>,
  toolId: string,
): AgentToolBinding {
  return tools[toolId] ?? { enabled: false, last_applied_preset: "ALL" };
}

function AgentProfileFields({
  draft,
  saveBlocked,
  onChange,
}: {
  draft: AgentProfile;
  saveBlocked: boolean;
  onChange: (profile: AgentProfile) => void;
}) {
  return (
    <div className="settings-card space-y-2 p-3">
      <div>
        <FieldLabel>Description</FieldLabel>
        <TextInput
          value={draft.description}
          onChange={(e) => onChange({ ...draft, description: e.target.value })}
          disabled={saveBlocked}
        />
      </div>
      <div>
        <FieldLabel>System prompt</FieldLabel>
        <TextArea
          rows={4}
          value={draft.system_prompt}
          onChange={(e) => onChange({ ...draft, system_prompt: e.target.value })}
          disabled={saveBlocked}
        />
      </div>
      <div>
        <FieldLabel>Max steps</FieldLabel>
        <TextInput
          type="number"
          min="1"
          value={draft.max_steps}
          onChange={(e) =>
            onChange({
              ...draft,
              max_steps: Number(e.target.value) || 1,
            })
          }
          disabled={saveBlocked}
        />
      </div>
    </div>
  );
}

function AllowedSubagentsSelect({
  draft,
  subagentIds,
  agents,
  saveBlocked,
  onChange,
}: {
  draft: AgentProfile;
  subagentIds: string[];
  agents: Record<string, AgentProfile>;
  saveBlocked: boolean;
  onChange: (profile: AgentProfile) => void;
}) {
  const launchEnabled = draft.tools.subagent_launch?.enabled === true;
  const allowed = new Set(draft.allowed_subagents ?? []);

  const toggle = (id: string) => {
    const next = new Set(allowed);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    onChange({ ...draft, allowed_subagents: [...next].sort() });
  };

  return (
    <div className="space-y-2">
        <h3 className="settings-section-title">Allowed subagents</h3>
        {launchEnabled && subagentIds.length > 0 && allowed.size === 0 ? (
        <p className="text-xs text-(--_dk-amber-500)">
          subagent_launch is enabled but no subagents are allowed — launches will fail until you
          select at least one.
        </p>
      ) : null}
      {subagentIds.length === 0 ? (
        <p className="text-sm text-(--_dk-text-disabled)">No subagent profiles defined yet.</p>
      ) : (
        <div className="settings-card max-h-48 space-y-1 overflow-y-auto p-3">
          {subagentIds.map((id) => (
            <label key={id} className="flex cursor-pointer items-start gap-2 text-sm">
              <input
                type="checkbox"
                checked={allowed.has(id)}
                disabled={saveBlocked}
                onChange={() => toggle(id)}
                className="mt-0.5 accent-(--_dk-accent-hover)"
              />
              <span>
                <span className="font-mono text-(--_dk-text-secondary)">{id}</span>
                {agents[id]?.description ? (
                  <span className="ml-2 text-xs text-(--_dk-text-disabled)">{agents[id].description}</span>
                ) : null}
              </span>
            </label>
          ))}
        </div>
      )}
    </div>
  );
}

function SubagentToolsMultiSelect({
  draft,
  bindableTools,
  saveBlocked,
  onChange,
}: {
  draft: AgentProfile;
  bindableTools: ToolCatalogEntry[];
  saveBlocked: boolean;
  onChange: (profile: AgentProfile) => void;
}) {
  const enabledIds = new Set(
    Object.entries(draft.tools)
      .filter(([, b]) => b.enabled)
      .map(([id]) => id),
  );

  const toggle = (toolId: string) => {
    const wasEnabled = enabledIds.has(toolId);
    onChange({
      ...draft,
      tools: applyToolEnabled(draft.tools, toolId, !wasEnabled),
    });
  };

  if (bindableTools.length === 0) {
    return (
      <p className="text-sm text-(--_dk-amber-500)">
        No tools in catalog — enable optional or custom tools in Tool Catalog first.
      </p>
    );
  }

  return (
    <div className="space-y-2">
        <h3 className="settings-section-title">Tool bindings</h3>
        <div className="settings-card max-h-64 space-y-1 overflow-y-auto p-3">
        {bindableTools.map((entry) => (
          <label key={entry.id} className="flex cursor-pointer items-center gap-2 text-sm">
            <input
              type="checkbox"
              checked={enabledIds.has(entry.id)}
              disabled={saveBlocked}
              onChange={() => toggle(entry.id)}
              className="accent-(--_dk-accent-hover)"
            />
            <span className="font-mono text-(--_dk-text-secondary)">{entry.id}</span>
            <span className="text-dk-xs text-(--_dk-text-disabled)">{entry.tier}</span>
          </label>
        ))}
      </div>
    </div>
  );
}

export function AgentToolsGrid({
  draft,
  bindableTools,
  saveBlocked,
  onBindingChange,
  gridStyle,
}: {
  draft: AgentProfile;
  bindableTools: ToolCatalogEntry[];
  saveBlocked: boolean;
  onBindingChange: (toolId: string, patch: Partial<AgentToolBinding>) => void;
  /** Inline style for the card grid (e.g. force columns below sm breakpoint). */
  gridStyle?: CSSProperties;
}) {
  if (bindableTools.length === 0) {
    return (
      <div className="space-y-2">
        <h3 className="settings-section-title">Tool bindings</h3>
        <p className="text-sm text-(--_dk-amber-500)">No tools in catalog.</p>
      </div>
    );
  }

  return (
    <div className="space-y-2">
      <h3 className="settings-section-title">Tool bindings</h3>
      <div
        className="grid grid-cols-1 gap-3 p-3 sm:grid-cols-2 xl:grid-cols-3"
        style={gridStyle}
        role="list"
        aria-label="Agent tool bindings"
      >
        {bindableTools.map((entry) => {
          const binding = bindingFor(draft.tools, entry.id);
          const configurable = isConfigurableTool(entry.id);
          const enabled = binding.enabled;
          const preset = binding.last_applied_preset ?? "ALL";

          const toggleEnabled = () => {
            if (saveBlocked) return;
            onBindingChange(entry.id, { enabled: !enabled });
          };

          return (
            <div
              key={entry.id}
              data-enabled={enabled ? "true" : "false"}
              className="tool-binding-card flex flex-col overflow-hidden"
            >
              <button
                type="button"
                disabled={saveBlocked}
                aria-pressed={enabled}
                aria-label={`${entry.id} tool binding, ${enabled ? "enabled" : "disabled"}. Click the card to toggle.`}
                className="tool-binding-toggle"
                onClick={toggleEnabled}
              />
              <div className="tool-binding-content flex flex-col">
                <div className="flex w-full items-start justify-between gap-2 p-3">
                  <div className="min-w-0">
                    <p className="tool-binding-title truncate font-mono text-sm text-(--_dk-text-primary)">{entry.id}</p>
                    <p className="mt-0.5 text-dk-xs text-(--_dk-text-disabled)">{entry.tier}</p>
                  </div>
                  <span className={`tag ${enabled ? "tag-ok" : "tag-neutral"} tag-sm tag-outline`}>
                    {enabled ? "On" : "Off"}
                  </span>
                </div>

                <div className="tool-binding-foot px-3 py-2.5">
                  {configurable ? (
                    <div
                      className="flex flex-wrap gap-1.5"
                      role="group"
                      aria-label={`${entry.id} preset`}
                    >
                      {(["ALL", "SAFE"] as const).map((value) => (
                        <button
                          key={value}
                          type="button"
                          disabled={saveBlocked || !enabled}
                          aria-pressed={preset === value}
                          onClick={(event) => {
                            event.stopPropagation();
                            onBindingChange(entry.id, { last_applied_preset: value });
                          }}
                          className={[
                            // Only intercept hits when the control is live; disabled
                            // buttons stay click-through so the full-card toggle works.
                            enabled && !saveBlocked ? "tool-binding-action" : "",
                            preset === value ? "btn-primary btn-xs" : "btn-ghost btn-xs",
                          ]
                            .filter(Boolean)
                            .join(" ")}
                        >
                          {value}
                        </button>
                      ))}
                    </div>
                  ) : (
                    <p className="text-xs text-(--_dk-text-disabled)">
                      {entry.tier === "mcp" ? "No preset" : "Not configurable"}
                    </p>
                  )}
                </div>
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}

// Temperature is built-in: modern providers ignore it, so we never expose it
// and always persist a fixed default rather than a user-edited value.
const BUILTIN_TEMPERATURE = 0.7;

function agentPersistPayload(
  draft: AgentProfile,
  selectedAgentId: string,
  bindableToolsPrimary: ToolCatalogEntry[],
  bindableToolsSubagent: ToolCatalogEntry[],
): AgentProfile {
  const isHidden = isHiddenSettingsAgent(selectedAgentId, draft.role);
  const bindableTools =
    draft.role === "subagent" ? bindableToolsSubagent : bindableToolsPrimary;
  const bindableIds = new Set(bindableTools.map((entry) => entry.id));
  const tools = Object.fromEntries(
    Object.entries(draft.tools).filter(([id]) => bindableIds.has(id)),
  );
  if (isHidden) {
    return { ...draft, tools: {}, allowed_subagents: [], temperature: BUILTIN_TEMPERATURE };
  }
  if (draft.role === "subagent") {
    return withSyncedToolSeries({
      ...draft,
      tools,
      allowed_subagents: [],
      temperature: BUILTIN_TEMPERATURE,
    });
  }
  return withSyncedToolSeries({ ...draft, tools, temperature: BUILTIN_TEMPERATURE });
}

export function AgentsSection() {
  const toolCatalog = useSettingsStore((s) => s.toolCatalog);
  const models = useSettingsStore((s) => s.models);
  const agentIds = useSettingsStore((s) => s.agentIds);
  const selectedAgentId = useSettingsStore((s) => s.selectedAgentId);
  const agents = useSettingsStore((s) => s.agents);
  const saving = useSettingsStore((s) => s.saving);
  const saveBlocked = useSettingsStore((s) => s.isSaveBlocked());
  const persistStatus = useSettingsStore((s) => s.persistStatus);
  const setPersistStatus = useSettingsStore((s) => s.setPersistStatus);
  const setSelectedAgentId = useSettingsStore((s) => s.setSelectedAgentId);
  const saveAgent = useSettingsStore((s) => s.saveAgent);
  const createAgent = useSettingsStore((s) => s.createAgent);
  const removeAgent = useSettingsStore((s) => s.removeAgent);
  const refreshAgents = useSettingsStore((s) => s.refreshAgents);

  const [creating, setCreating] = useState(false);
  const [newAgentId, setNewAgentId] = useState("");

  const profile = agents[selectedAgentId];
  const [draft, setDraft] = useState<AgentProfile | null>(null);

  const subagentIds = useMemo(
    () =>
      agentIds.filter((id) => agents[id]?.role === "subagent").sort((a, b) => a.localeCompare(b)),
    [agentIds, agents],
  );

  const bindableToolsPrimary = useMemo(() => {
    if (!toolCatalog) return [];
    return Object.values(toolCatalog)
      .filter(isAgentBindableTool)
      .sort((a, b) => {
        const tierOrder = (tier: ToolCatalogEntry["tier"]) => {
          if (tier === "core") return 0;
          if (tier === "optional") return 1;
          if (tier === "custom") return 2;
          if (tier === "mcp") return 3;
          return 9;
        };
        const byTier = tierOrder(a.tier) - tierOrder(b.tier);
        return byTier !== 0 ? byTier : a.id.localeCompare(b.id);
      });
  }, [toolCatalog]);

  const bindableToolsSubagent = useMemo(() => {
    if (!toolCatalog) return [];
    return Object.values(toolCatalog)
      .filter(isSubagentBindableTool)
      .sort((a, b) => a.id.localeCompare(b.id));
  }, [toolCatalog]);

  useEffect(() => {
    if (!profile || creating) return;
    if (isPersistBusy(persistStatus)) return;
    const bindableIds = new Set(
      (profile.role === "subagent" ? bindableToolsSubagent : bindableToolsPrimary).map(
        (e) => e.id,
      ),
    );
    const tools = Object.fromEntries(
      Object.entries(profile.tools).filter(([id]) => bindableIds.has(id)),
    );
    setDraft({
      ...profile,
      allowed_subagents: profile.allowed_subagents ?? [],
      tools,
    });
  }, [profile, selectedAgentId, bindableToolsPrimary, bindableToolsSubagent, persistStatus, creating]);

  useSettingsPersist(draft, {
    enabled: !creating && draft != null,
    debounceMs: 400,
    setStatus: setPersistStatus,
    serialize: (d) => {
      if (!d) return { skip: "unchanged" };
      return {
        ok: agentPersistPayload(
          d,
          selectedAgentId,
          bindableToolsPrimary,
          bindableToolsSubagent,
        ),
      };
    },
    commit: (p) => saveAgent(selectedAgentId, p),
    revert: () => {
      const snap = useSettingsStore.getState();
      const current = snap.agents[snap.selectedAgentId];
      if (!current) return;
      setDraft({
        ...current,
        allowed_subagents: current.allowed_subagents ?? [],
      });
    },
  });

  const isHiddenAgent =
    profile != null && isHiddenSettingsAgent(selectedAgentId, profile.role);

  const updateBinding = (toolId: string, patch: Partial<AgentToolBinding>) => {
    if (!draft) return;
    if (patch.enabled !== undefined) {
      setDraft({
        ...draft,
        tools: applyToolEnabled(draft.tools, toolId, patch.enabled),
      });
      return;
    }
    const current = bindingFor(draft.tools, toolId);
    setDraft({
      ...draft,
      tools: {
        ...draft.tools,
        [toolId]: { ...current, ...patch },
      },
    });
  };

  const startCreate = () => {
    setCreating(true);
    setNewAgentId("");
    setDraft({
      role: "subagent",
      model_ref: Object.keys(models ?? {})[0] ?? "",
      system_prompt: "",
      temperature: BUILTIN_TEMPERATURE,
      max_steps: 50,
      description: "",
      tools: {},
      allowed_subagents: [],
    });
  };

  const cancelCreate = () => {
    setCreating(false);
    setNewAgentId("");
    if (profile) {
      setDraft({ ...profile, allowed_subagents: profile.allowed_subagents ?? [] });
    }
  };

  const onCreate = () => {
    if (!draft || !newAgentId.trim()) return;
    const payload = agentPersistPayload(
      draft,
      newAgentId.trim(),
      bindableToolsPrimary,
      bindableToolsSubagent,
    );
    void createAgent(newAgentId.trim(), payload)
      .then(() => {
        setCreating(false);
        setPersistStatus("saved");
      })
      .catch(() => {
        // toast already shown
      });
  };

  const onDelete = () => {
    if (isProtectedAgent(selectedAgentId)) return;
    if (!window.confirm(`Delete agent "${selectedAgentId}"?`)) return;
    void removeAgent(selectedAgentId);
  };

  if (!draft) {
    return (
      <SettingsPageShell title="Agents">
        <p className="text-sm text-(--_dk-text-disabled)">Loading agents…</p>
      </SettingsPageShell>
    );
  }

  return (
    <SettingsPageShell
      title="Agents"
      actions={
        <>
          {creating ? (
            <button
              type="button"
              onClick={onCreate}
              disabled={saveBlocked || !newAgentId.trim() || saving}
              className="btn-primary btn-sm"
            >
              Create
            </button>
          ) : null}
          <button
            type="button"
            onClick={startCreate}
            disabled={saveBlocked}
            className="btn btn-icon"
            aria-label="Add agent"
            title="Add agent"
          >
            <Plus size={16} />
          </button>
          <button
            type="button"
            onClick={() => void refreshAgents()}
            className="btn btn-icon"
            aria-label="Refresh list"
            title="Refresh list"
          >
            <ArrowClockwise size={16} />
          </button>
        </>
      }
    >
      <div className="space-y-6">
      <div className="space-y-3">
        {creating ? (
          <div className="settings-card space-y-3 p-3">
            <div>
              <FieldLabel required>New agent id</FieldLabel>
              <TextInput
                value={newAgentId}
                onChange={(e) => setNewAgentId(e.target.value.toLowerCase())}
                placeholder="my_agent"
                disabled={saveBlocked}
                className="font-mono"
              />
              <p className="settings-field-hint">Lowercase letters, digits, and underscores.</p>
            </div>
            <div className="flex flex-wrap gap-2">
              <button type="button" onClick={cancelCreate} className="btn-ghost">
                Cancel
              </button>
            </div>
          </div>
        ) : (
          <div className="settings-card space-y-3 p-3">
            <div className="flex flex-wrap items-end gap-3">
              <div className="min-w-[160px] flex-1">
                <FieldLabel>Agent</FieldLabel>
                <Select
                  value={selectedAgentId}
                  onChange={(id) => {
                    void flushRegisteredSettings().then(() => setSelectedAgentId(id));
                  }}
                  options={agentIds.map((id) => {
                    const role = agents[id]?.role ?? "primary";
                    return {
                      value: id,
                      label: (
                        <span className="flex items-center gap-1.5">
                          <AgentTypeIcon role={role} color={agentColor(id)} />
                          <span>{id}</span>
                          {role === "hidden" ? (
                            <span className="text-(--_dk-text-disabled)">(hidden)</span>
                          ) : null}
                        </span>
                      ),
                    };
                  })}
                  className="w-full"
                />
              </div>
              <div className="min-w-[160px] flex-1">
                <FieldLabel>Model</FieldLabel>
                <ModelRefSelect
                  value={draft.model_ref}
                  models={models}
                  onChange={(model_ref) => setDraft({ ...draft, model_ref })}
                  disabled={saveBlocked}
                />
              </div>
              <div>
                <FieldLabel>Type</FieldLabel>
                <div className="flex gap-2">
                  {(["primary", "subagent"] as const).map((role) => (
                    <button
                      key={role}
                      type="button"
                      disabled={saveBlocked}
                      onClick={() => setDraft({ ...draft, role })}
                      className={`flex items-center gap-1 ${draft.role === role ? "btn-primary btn-xs" : "btn-ghost btn-xs"}`}
                    >
                      <AgentTypeIcon role={role} />
                      {role === "primary" ? "Primary" : "Subagent"}
                    </button>
                  ))}
                </div>
              </div>
            </div>
            {!isProtectedAgent(selectedAgentId) ? (
              <div className="flex justify-end">
                <button
                  type="button"
                  onClick={onDelete}
                  disabled={saveBlocked || saving}
                  className="btn-danger btn-xs"
                >
                  Delete
                </button>
              </div>
            ) : null}
          </div>
        )}

        <AgentProfileFields
          draft={draft}
          saveBlocked={saveBlocked}
          onChange={setDraft}
        />
      </div>

      {!isHiddenAgent && draft.role === "primary" ? (
        <>
          <AllowedSubagentsSelect
            draft={draft}
            subagentIds={subagentIds}
            agents={agents}
            saveBlocked={saveBlocked}
            onChange={setDraft}
          />
          <AgentToolsGrid
              draft={draft}
              bindableTools={bindableToolsPrimary}
              saveBlocked={saveBlocked}
              onBindingChange={updateBinding}
            />
        </>
      ) : !isHiddenAgent && draft.role === "subagent" ? (
        <SubagentToolsMultiSelect
          draft={draft}
          bindableTools={bindableToolsSubagent}
          saveBlocked={saveBlocked}
          onChange={setDraft}
        />
      ) : isHiddenAgent ? (
        <p className="text-xs text-(--_dk-text-disabled)">
          Hidden agents have no tool list (compaction uses model and prompt only).
        </p>
      ) : null}
      </div>
    </SettingsPageShell>
  );
}

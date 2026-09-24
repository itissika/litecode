import {
  useEffect,
  useMemo,
  useState,
  type CSSProperties,
  type ReactNode,
} from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { Plus, ArrowClockwise, ListChecks } from "@phosphor-icons/react";

import {
  isConfigurableTool,
  isHiddenSettingsAgent,
  isProtectedAgent,
  isSubagentBindableTool,
  modelRefLabel,
  splitModelRef,
  applyToolEnabled,
  withSyncedToolSeries,
  type AgentProfile,
  type AgentToolBinding,
  type AvailableTool,
  type CatalogModelDto,
  type CatalogProviderDto,
} from "../../../api/settings";
import { useSettingsStore } from "../../../stores/settingsStore";
import { mergeLayeredMcp } from "../../../stores/settingsDocuments";
import { Select } from "../../../components/ui/Select";
import { Dropdown } from "../../../components/ui/Dropdown";
import { FoldCard } from "../../../components/FoldCard";
import { ProviderLogo } from "../../../components/ProviderLogos";
import { AgentTypeIcon, agentColor } from "../../../components/agentIdentity";
import {
  FieldLabel,
  TextInput,
  TextArea,
  SettingsPageShell,
  useSettingsSaveBlocked,
} from "./shared";
import {
  flushRegisteredSettings,
  isPersistBusy,
  shouldHydrateDraftFromStore,
  useDocPersist,
  useSettingsPersist,
} from "./persist";

type ModelRefOption = { value: string; label: ReactNode; disabled?: boolean };

/**
 * Agent model picker over the **active** catalog models (their provider holds a
 * credential), grouped by provider. A `model_ref` that is not in the catalog
 * stays visible as a disabled "Missing: <ref>" row so the user can re-pick it;
 * the backend may meanwhile auto-heal the agent to the first runnable model.
 */
function ModelRefSelect({
  value,
  models,
  providers,
  onChange,
  disabled,
}: {
  value: string;
  models: CatalogModelDto[];
  providers: CatalogProviderDto[];
  onChange: (modelRef: string) => void;
  disabled?: boolean;
}) {
  const options = useMemo(() => {
    const providerName = new Map(providers.map((p) => [p.id, p.name]));
    const nameOf = (id: string) => providerName.get(id) ?? id;
    const known = new Set(models.map((m) => m.ref));

    const rows: ModelRefOption[] = [
      { value: "", label: "— select —" as ReactNode },
    ];
    if (value && !known.has(value)) {
      // Missing first, marked in amber: the current value must stay visible.
      const { providerId } = splitModelRef(value);
      const owner = providerId ? nameOf(providerId) : "";
      rows.push({
        value,
        disabled: true,
        label: (
          <span className="flex items-center justify-between gap-2 text-(--_dk-amber-500)">
            <span className="flex min-w-0 items-center gap-1.5">
              <ProviderLogo providerId={providerId} />
              <span className="truncate">Missing: {value}</span>
            </span>
            {owner ? (
              <span className="shrink-0 text-dk-xs opacity-70">{owner}</span>
            ) : null}
          </span>
        ),
      });
    }

    // Group by provider; catalog order is preserved inside each provider.
    const groups = new Map<string, CatalogModelDto[]>();
    for (const model of models) {
      const bucket = groups.get(model.provider_id);
      if (bucket) bucket.push(model);
      else groups.set(model.provider_id, [model]);
    }
    for (const [providerId, list] of groups) {
      rows.push({
        value: `__provider__${providerId}`,
        disabled: true,
        label: (
          <span className="text-dk-xs uppercase tracking-wide text-(--_dk-text-disabled)">
            {nameOf(providerId)}
          </span>
        ),
      });
      for (const model of list) {
        // The brand mark rides on every row so two providers offering the same
        // wire model id stay distinguishable (also in the collapsed trigger,
        // which renders the selected option's label).
        rows.push({
          value: model.ref,
          label: (
            <span className="flex min-w-0 items-center gap-1.5">
              <ProviderLogo providerId={model.provider_id} />
              <span className="truncate">{modelRefLabel(model)}</span>
            </span>
          ),
        });
      }
    }
    return rows;
  }, [models, providers, value]);

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

function McpToolVisibilityControl({
  serverId,
  binding,
  tools,
  disabled,
  onChange,
}: {
  serverId: string;
  binding: AgentToolBinding;
  tools: { name: string; description: string }[];
  disabled: boolean;
  onChange: (allowed_tools: string[]) => void;
}) {
  const selected =
    binding.allowed_tools == null
      ? new Set(tools.map((tool) => tool.name))
      : new Set(binding.allowed_tools);

  const toggleTool = (name: string) => {
    const next = new Set(selected);
    if (next.has(name)) next.delete(name);
    else next.add(name);
    onChange(
      tools.filter((tool) => next.has(tool.name)).map((tool) => tool.name),
    );
  };

  // Virtualize the tool list: MCP servers can expose hundreds of tools, so
  // only the visible rows are mounted (same tanstack virtualizer as MessageList).
  // The scroll element lives inside the Dropdown's portal and only mounts once
  // the panel opens, so it's stored via a callback ref → state: that forces a
  // re-render when it mounts, letting the virtualizer attach its observers.
  const [scrollEl, setScrollEl] = useState<HTMLDivElement | null>(null);
  const virtualizer = useVirtualizer({
    count: tools.length,
    getScrollElement: () => scrollEl,
    estimateSize: () => 32,
    overscan: 6,
  });

  return (
    <Dropdown
      variant="panel"
      align="right"
      flip
      closeOnSelect={false}
      panelClassName="w-[360px] max-w-[calc(100vw-16px)] p-2"
      trigger={({ open, toggle }) => (
        <button
          type="button"
          disabled={disabled}
          aria-label={`Select visible tools for MCP server ${serverId}`}
          aria-expanded={open}
          className={[
            disabled ? "" : "tool-binding-action",
            "btn btn-ghost btn-xs",
          ]
            .filter(Boolean)
            .join(" ")}
          onClick={(event) => {
            event.stopPropagation();
            toggle();
          }}
        >
          <ListChecks size={14} />
          Tools
        </button>
      )}
    >
      <div className="space-y-1">
        <p className="px-1 pb-1 text-xs text-(--_dk-text-muted)">
          Visible to the model next turn
        </p>
        {tools.length === 0 ? (
          <p className="px-1 py-2 text-xs text-(--_dk-text-disabled)">
            No tools listed. Start this MCP server from the MCP settings page.
          </p>
        ) : (
          <div ref={setScrollEl} className="max-h-64 overflow-y-auto">
            <div
              style={{
                height: `${virtualizer.getTotalSize()}px`,
                position: "relative",
                width: "100%",
              }}
            >
              {virtualizer.getVirtualItems().map((vi) => {
                const tool = tools[vi.index];
                const checked = selected.has(tool.name);
                return (
                  <div
                    key={vi.key}
                    data-index={vi.index}
                    ref={virtualizer.measureElement}
                    className="py-1"
                    style={{
                      position: "absolute",
                      top: 0,
                      left: 0,
                      width: "100%",
                      transform: `translateY(${vi.start}px)`,
                    }}
                  >
                    <FoldCard
                      label={
                        <label
                          className="flex min-w-0 flex-1 cursor-pointer items-center gap-2"
                          onClick={(event) => event.stopPropagation()}
                        >
                          <input
                            type="checkbox"
                            checked={checked}
                            disabled={disabled}
                            onChange={() => toggleTool(tool.name)}
                            className="accent-(--_dk-accent-hover)"
                          />
                          <span className="min-w-0 truncate font-mono text-xs text-(--_dk-text-primary)">
                            {tool.name}
                          </span>
                        </label>
                      }
                      headerAriaLabel={`Show details for MCP tool ${tool.name}`}
                      className="border-b border-(--_dk-line)"
                      contentClassName="px-5 pb-2 text-xs text-(--_dk-text-secondary)"
                    >
                      {tool.description ||
                        "No description provided by this MCP server."}
                    </FoldCard>
                  </div>
                );
              })}
            </div>
          </div>
        )}
      </div>
    </Dropdown>
  );
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
          onChange={(e) =>
            onChange({ ...draft, system_prompt: e.target.value })
          }
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
          subagent_launch is enabled but no subagents are allowed — launches
          will fail until you select at least one.
        </p>
      ) : null}
      {subagentIds.length === 0 ? (
        <p className="text-sm text-(--_dk-text-disabled)">
          No subagent profiles defined yet.
        </p>
      ) : (
        <div className="settings-card max-h-48 space-y-1 overflow-y-auto p-3">
          {subagentIds.map((id) => (
            <label
              key={id}
              className="flex cursor-pointer items-start gap-2 text-sm"
            >
              <input
                type="checkbox"
                checked={allowed.has(id)}
                disabled={saveBlocked}
                onChange={() => toggle(id)}
                className="mt-0.5 accent-(--_dk-accent-hover)"
              />
              <span>
                <span className="font-mono text-(--_dk-text-secondary)">
                  {id}
                </span>
                {agents[id]?.description ? (
                  <span className="ml-2 text-xs text-(--_dk-text-disabled)">
                    {agents[id].description}
                  </span>
                ) : null}
              </span>
            </label>
          ))}
        </div>
      )}
    </div>
  );
}

export function AgentToolsGrid({
  draft,
  bindableTools,
  mcpServers,
  saveBlocked,
  onBindingChange,
  gridStyle,
  note,
}: {
  draft: AgentProfile;
  bindableTools: AvailableTool[];
  mcpServers: { id: string; tools?: { name: string; description: string }[] }[];
  saveBlocked: boolean;
  onBindingChange: (toolId: string, patch: Partial<AgentToolBinding>) => void;
  /** Inline style for the card grid (e.g. force columns below sm breakpoint). */
  gridStyle?: CSSProperties;
  /** Extra guidance rendered under the section title (e.g. subagent deny semantics). */
  note?: ReactNode;
}) {
  if (bindableTools.length === 0) {
    return (
      <div className="space-y-2">
        <h3 className="settings-section-title">Tool bindings</h3>
        {note ? (
          <p className="text-xs text-(--_dk-text-muted)">{note}</p>
        ) : null}
        <p className="text-sm text-(--_dk-amber-500)">
          No bindable tools in this workspace.
        </p>
      </div>
    );
  }

  return (
    <div className="space-y-2">
      <h3 className="settings-section-title">Tool bindings</h3>
      {note ? <p className="text-xs text-(--_dk-text-muted)">{note}</p> : null}
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
          const serverId = entry.id.slice("mcp_".length);
          const mcpTools =
            entry.kind === "mcp"
              ? (mcpServers.find((server) => server.id === serverId)?.tools ??
                [])
              : [];

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
                    <p className="tool-binding-title truncate font-mono text-sm text-(--_dk-text-primary)">
                      {entry.id}
                    </p>
                    <p className="mt-0.5 text-dk-xs text-(--_dk-text-disabled)">
                      {entry.kind}
                      {entry.overridden ? " · workspace override" : ""}
                    </p>
                  </div>
                  <span
                    className={`tag ${enabled ? "tag-ok" : "tag-neutral"} tag-sm tag-outline`}
                  >
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
                            onBindingChange(entry.id, {
                              last_applied_preset: value,
                            });
                          }}
                          className={[
                            // Only intercept hits when the control is live; disabled
                            // buttons stay click-through so the full-card toggle works.
                            enabled && !saveBlocked
                              ? "tool-binding-action"
                              : "",
                            preset === value
                              ? "btn-primary btn-xs"
                              : "btn-ghost btn-xs",
                          ]
                            .filter(Boolean)
                            .join(" ")}
                        >
                          {value}
                        </button>
                      ))}
                    </div>
                  ) : (
                    <div className="flex items-center justify-between gap-2">
                      <p className="text-xs text-(--_dk-text-disabled)">
                        {entry.kind === "mcp"
                          ? "No preset"
                          : "Not configurable"}
                      </p>
                      {entry.kind === "mcp" ? (
                        <McpToolVisibilityControl
                          serverId={serverId}
                          binding={binding}
                          tools={mcpTools}
                          disabled={saveBlocked || !enabled}
                          onChange={(allowed_tools) =>
                            onBindingChange(entry.id, { allowed_tools })
                          }
                        />
                      ) : null}
                    </div>
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

// Bindings for tools not in this workspace catalog stay on the profile so a PUT
// does not wipe other-workspace MCP/custom rows. The grid only shows catalog ids.
function agentPersistPayload(
  draft: AgentProfile,
  selectedAgentId: string,
): AgentProfile {
  const isHidden = isHiddenSettingsAgent(selectedAgentId, draft.role);
  if (isHidden) {
    return {
      ...draft,
      role: "hidden",
      tools: {},
      allowed_subagents: [],
      temperature: BUILTIN_TEMPERATURE,
    };
  }
  if (draft.role === "subagent") {
    return withSyncedToolSeries({
      ...draft,
      tools: { ...draft.tools },
      allowed_subagents: [],
      temperature: BUILTIN_TEMPERATURE,
    });
  }
  return withSyncedToolSeries({
    ...draft,
    tools: { ...draft.tools },
    temperature: BUILTIN_TEMPERATURE,
  });
}

export function AgentsSection() {
  const availableTools = useSettingsStore((s) => s.availableTools);
  const llm = useSettingsStore((s) => s.llm);
  const activeModels = useMemo(() => llm?.active_models ?? [], [llm]);
  const catalogProviders = useMemo(() => llm?.providers ?? [], [llm]);
  const mcpDefs = useSettingsStore((s) => s.mcpDefs);
  const mcpRuntime = useSettingsStore((s) => s.mcpRuntime);
  const mcpList = useMemo(() => {
    const layered = mergeLayeredMcp(mcpDefs, mcpRuntime);
    return [...layered.global, ...layered.workspace];
  }, [mcpDefs, mcpRuntime]);
  const agentIds = useSettingsStore((s) => s.agentIds);
  const selectedAgentId = useSettingsStore((s) => s.selectedAgentId);
  const agents = useSettingsStore((s) => s.agents);
  const saveBlocked = useSettingsSaveBlocked();
  const { persistStatus, setPersistStatus } = useDocPersist("agents");
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
      agentIds
        .filter((id) => agents[id]?.role === "subagent")
        .sort((a, b) => a.localeCompare(b)),
    [agentIds, agents],
  );

  const bindableToolsPrimary = useMemo(() => {
    if (!availableTools) return [];
    return [...availableTools].sort((a, b) => {
      const kindOrder = (kind: AvailableTool["kind"]) => {
        if (kind === "core") return 0;
        if (kind === "engine") return 1;
        if (kind === "custom") return 2;
        if (kind === "mcp") return 3;
        return 9;
      };
      const byKind = kindOrder(a.kind) - kindOrder(b.kind);
      return byKind !== 0 ? byKind : a.id.localeCompare(b.id);
    });
  }, [availableTools]);

  const bindableToolsSubagent = useMemo(() => {
    if (!availableTools) return [];
    return availableTools
      .filter(isSubagentBindableTool)
      .sort((a, b) => a.id.localeCompare(b.id));
  }, [availableTools]);

  useEffect(() => {
    if (!profile || creating) return;
    if (!shouldHydrateDraftFromStore(persistStatus)) return;
    setDraft({
      ...profile,
      allowed_subagents: profile.allowed_subagents ?? [],
      tools: { ...profile.tools },
    });
  }, [profile, selectedAgentId, persistStatus, creating]);

  useSettingsPersist<AgentProfile | null, AgentProfile>(draft, {
    enabled: !creating && draft != null,
    debounceMs: 400,
    setStatus: setPersistStatus,
    serialize: (d) => {
      if (!d) return { skip: "unchanged" };
      return {
        ok: agentPersistPayload(d, selectedAgentId),
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
      // A new agent starts on the first active catalog model.
      model_ref: activeModels[0]?.ref ?? "",
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
      setDraft({
        ...profile,
        allowed_subagents: profile.allowed_subagents ?? [],
      });
    }
  };

  const onCreate = () => {
    if (!draft || !newAgentId.trim()) return;
    const payload = agentPersistPayload(draft, newAgentId.trim());
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
              disabled={
                saveBlocked ||
                !newAgentId.trim() ||
                isPersistBusy(persistStatus)
              }
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
                <p className="settings-field-hint">
                  Lowercase letters, digits, and underscores.
                </p>
              </div>
              <div className="flex flex-wrap gap-2">
                <button
                  type="button"
                  onClick={cancelCreate}
                  className="btn-ghost"
                >
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
                      void flushRegisteredSettings().then(() =>
                        setSelectedAgentId(id),
                      );
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
                              <span className="text-(--_dk-text-disabled)">
                                (hidden)
                              </span>
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
                    models={activeModels}
                    providers={catalogProviders}
                    onChange={(model_ref) => setDraft({ ...draft, model_ref })}
                    disabled={saveBlocked}
                  />
                  {activeModels.length === 0 ? (
                    <p className="settings-field-hint">
                      Configure a provider API key
                    </p>
                  ) : null}
                </div>
                {!isHiddenAgent ? (
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
                ) : null}
              </div>
              {!isProtectedAgent(selectedAgentId) ? (
                <div className="flex justify-end">
                  <button
                    type="button"
                    onClick={onDelete}
                    disabled={saveBlocked || isPersistBusy(persistStatus)}
                    className="btn-danger btn-xs"
                  >
                    Delete
                  </button>
                </div>
              ) : null}
            </div>
          )}

          {creating || !isHiddenAgent ? (
            <AgentProfileFields
              draft={draft}
              saveBlocked={saveBlocked}
              onChange={setDraft}
            />
          ) : null}
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
              mcpServers={mcpList}
              saveBlocked={saveBlocked}
              onBindingChange={updateBinding}
            />
          </>
        ) : !isHiddenAgent && draft.role === "subagent" ? (
          <AgentToolsGrid
            draft={draft}
            bindableTools={bindableToolsSubagent}
            mcpServers={mcpList}
            saveBlocked={saveBlocked}
            onBindingChange={updateBinding}
            note="Subagents can't ask for approval — a binding that would prompt the user (SAFE on write/edit, web, or custom tools) is denied at runtime instead. SAFE on read/grep/glob/bash keeps them workspace-safe; use the MCP Tools picker to expose only chosen tools of a server."
          />
        ) : isHiddenAgent ? (
          <p className="text-xs text-(--_dk-text-disabled)">
            Compaction only assigns a model. Prompt, tools, and max steps are
            built in.
          </p>
        ) : null}
      </div>
    </SettingsPageShell>
  );
}

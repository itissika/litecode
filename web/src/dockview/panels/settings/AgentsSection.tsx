import { useEffect, useMemo, useState } from "react";
import { Plus, ArrowClockwise } from "@phosphor-icons/react";
import { useTranslation } from "react-i18next";

import {
  presetToolIds,
  settingsToolSwitches,
  isProtectedAgent,
  applyToolEnabled,
  type AgentProfile,
  type AgentToolBinding,
  type CatalogModelDto,
} from "../../../api/settings";
import type { ModelInfo } from "../../../api/types";
import { useSettingsStore } from "../../../stores/settingsStore";
import { mergeLayeredMcp } from "../../../stores/settingsDocuments";
import { ModelSwitcher } from "../../../components/ModelSwitcher";
import { Select } from "../../../components/ui/Select";
import { AgentTypeIcon, agentColor } from "../../../components/agentIdentity";
import { AgentProfileFields } from "../../../components/agent/AgentProfileFields";
import {
  AgentToolsGrid,
  bindingFor,
} from "../../../components/agent/AgentToolsGrid";
import {
  FieldLabel,
  TextInput,
  SettingsPageShell,
  useSettingsSaveBlocked,
} from "./shared";
import { useConfirmDialog } from "../../../components/ui/useConfirmDialog";
import {
  flushRegisteredSettings,
  isPersistBusy,
  shouldHydrateDraftFromStore,
  useDocPersist,
  useSettingsPersist,
} from "./persist";

/** Settings catalog row → the ref the shared picker stores and compares. */
function toSwitcherModel(model: CatalogModelDto): ModelInfo {
  return {
    id: model.ref,
    api_model_id: model.id,
    provider_id: model.provider_id,
    label: model.label,
    context_window: model.context_window,
    modalities: model.modalities,
  };
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
  const { t } = useTranslation();
  const launchEnabled =
    (draft.tools.subagent_tools ?? draft.tools.subagent_launch)?.enabled === true;
  const allowed = new Set(draft.allowed_subagents ?? []);

  const toggle = (id: string) => {
    const next = new Set(allowed);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    onChange({ ...draft, allowed_subagents: [...next].sort() });
  };

  return (
    <div className="space-y-2">
      <h3 className="settings-section-title">{t("settings.agents.allowedSubagents")}</h3>
      {launchEnabled && subagentIds.length > 0 && allowed.size === 0 ? (
        <p className="text-xs text-(--_dk-amber-500)">
          {t("settings.agents.selectSubagent")}
        </p>
      ) : null}
      {subagentIds.length === 0 ? (
        <p className="text-sm text-(--_dk-text-disabled)">
          {t("settings.agents.noSubagentProfiles")}
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

// Temperature is built-in: modern providers ignore it, so we never expose it
// and always persist a fixed default rather than a user-edited value.
const BUILTIN_TEMPERATURE = 0.7;

// Bindings for tools not in this workspace catalog stay on the profile so a PUT
// does not wipe other-workspace MCP/custom rows. The grid only shows catalog ids.
function agentPersistPayload(draft: AgentProfile): AgentProfile {
  if (draft.role === "subagent") {
    return {
      ...draft,
      tools: { ...draft.tools },
      allowed_subagents: [],
      temperature: BUILTIN_TEMPERATURE,
    };
  }
  return {
    ...draft,
    tools: { ...draft.tools },
    temperature: BUILTIN_TEMPERATURE,
  };
}

export function AgentsSection() {
  const { t } = useTranslation();
  const { confirm, dialog } = useConfirmDialog();
  const availableTools = useSettingsStore((s) => s.availableTools);
  const llm = useSettingsStore((s) => s.llm);
  const activeModels = useMemo(() => llm?.active_models ?? [], [llm]);
  const switcherModels = useMemo(
    () => activeModels.map(toSwitcherModel),
    [activeModels],
  );
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
  const focusAnchor = useSettingsStore((s) => s.focusAnchor);
  const consumeFocusAnchor = useSettingsStore((s) => s.consumeFocusAnchor);
  const saveAgent = useSettingsStore((s) => s.saveAgent);
  const createAgent = useSettingsStore((s) => s.createAgent);
  const removeAgent = useSettingsStore((s) => s.removeAgent);
  const refreshAgents = useSettingsStore((s) => s.refreshAgents);

  const [creating, setCreating] = useState(false);
  const [newAgentId, setNewAgentId] = useState("");
  /** Row id currently ringed + hinted (first-run Max steps detour). */
  const [flashedAnchor, setFlashedAnchor] = useState<string | null>(null);

  const profile = agents[selectedAgentId];
  const [draft, setDraft] = useState<AgentProfile | null>(null);

  const subagentIds = useMemo(
    () =>
      agentIds
        .filter((id) => agents[id]?.role === "subagent")
        .sort((a, b) => a.localeCompare(b)),
    [agentIds, agents],
  );

  const switchTools = useMemo(
    () => settingsToolSwitches(availableTools ?? []),
    [availableTools],
  );

  // Sectioning and ordering live in toolGroups: the grid groups by kind, so
  // neither list sorts here.
  const bindableToolsPrimary = switchTools;

  const bindableToolsSubagent = useMemo(
    () => switchTools.filter((tool) => tool.scope !== "primary_root_only"),
    [switchTools],
  );

  const presetIds = useMemo(
    () => presetToolIds(switchTools),
    [switchTools],
  );

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
        ok: agentPersistPayload(d),
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

  // The flash ends on its own: never a permanent decoration.
  useEffect(() => {
    if (!flashedAnchor) return;
    const timer = window.setTimeout(() => setFlashedAnchor(null), 8000);
    return () => window.clearTimeout(timer);
  }, [flashedAnchor]);

  /**
   * First-run deep-link (see lib/firstRun): centre the anchored row, ring it and
   * put the caret in its input. The value is the user's to change — this never
   * writes a setting.
   */
  useEffect(() => {
    if (focusAnchor !== "max-steps") return;
    // The row only exists once the draft is hydrated from the store.
    if (creating || !profile || !draft) return;
    consumeFocusAnchor();
    if (selectedAgentId !== "default") return;
    setFlashedAnchor("max-steps");
    const raf = requestAnimationFrame(() => {
      const row = document.querySelector<HTMLElement>('[data-anchor="max-steps"]');
      row?.scrollIntoView?.({ block: "center" });
      row?.querySelector("input")?.focus();
    });
    return () => cancelAnimationFrame(raf);
  }, [focusAnchor, creating, profile, draft, selectedAgentId, consumeFocusAnchor]);

  const updateBinding = (toolId: string, patch: Partial<AgentToolBinding>) => {
    if (!draft) return;
    if (patch.enabled !== undefined) {
      setDraft({
        ...draft,
        tools: applyToolEnabled(
          draft.tools,
          toolId,
          patch.enabled,
          presetIds,
        ),
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
    const payload = agentPersistPayload(draft);
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
    void confirm({
      title: t("common.delete"),
      body: t("settings.agents.deleteConfirm", { id: selectedAgentId }),
      danger: true,
      confirmLabel: t("common.delete"),
      cancelLabel: t("common.cancel"),
    }).then((ok) => {
      if (ok) void removeAgent(selectedAgentId);
    });
  };

  if (!draft) {
    return (
      <SettingsPageShell title={t('settings.agents.title')}>
        <p className="text-sm text-(--_dk-text-disabled)">{t("settings.agents.loading")}</p>
      </SettingsPageShell>
    );
  }

  return (
    <SettingsPageShell
      title={t('settings.agents.title')}
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
              {t("common.create")}
            </button>
          ) : null}
          <button
            type="button"
            onClick={startCreate}
            disabled={saveBlocked}
            className="btn btn-icon"
            aria-label={t("settings.agents.add")}
            title={t("settings.agents.add")}
          >
            <Plus size={16} />
          </button>
          <button
            type="button"
            onClick={() => void refreshAgents()}
            className="btn btn-icon"
            aria-label={t("settings.agents.refresh")}
            title={t("settings.agents.refresh")}
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
                <FieldLabel required>{t("settings.agents.newId")}</FieldLabel>
                <TextInput
                  value={newAgentId}
                  onChange={(e) => setNewAgentId(e.target.value.toLowerCase())}
                  placeholder="my_agent"
                  disabled={saveBlocked}
                  className="font-mono"
                />
                <p className="settings-field-hint">
                  {t("settings.agents.idHint")}
                </p>
              </div>
              <div className="flex flex-wrap gap-2">
                <button
                  type="button"
                  onClick={cancelCreate}
                  className="btn-ghost"
                >
                  {t("common.cancel")}
                </button>
              </div>
            </div>
          ) : (
            <div className="settings-card space-y-3 p-3">
              <div className="flex flex-wrap items-end gap-3">
                <div className="min-w-[160px] flex-1">
                  <FieldLabel>{t("common.agent")}</FieldLabel>
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
                                {t("common.hidden")}
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
                  <FieldLabel>{t("common.model")}</FieldLabel>
                  <ModelSwitcher
                    variant="field"
                    models={switcherModels}
                    modelId={draft.model_ref || null}
                    disabled={saveBlocked}
                    onChange={(model_ref) => setDraft({ ...draft, model_ref })}
                  />
                  {activeModels.length === 0 ? (
                    <p className="settings-field-hint">
                      {t("settings.agents.configureProvider")}
                    </p>
                  ) : null}
                </div>
                <div>
                  <FieldLabel>{t("common.type")}</FieldLabel>
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
                        {role === "primary" ? t("settings.agents.primary") : t("settings.agents.subagentRole")}
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
                    disabled={saveBlocked || isPersistBusy(persistStatus)}
                    className="btn-danger btn-xs"
                  >
                    {t("common.delete")}
                  </button>
                </div>
              ) : null}
            </div>
          )}

          {creating || draft.role !== "hidden" ? (
            <AgentProfileFields
              draft={draft}
              saveBlocked={saveBlocked}
              onChange={setDraft}
              flashMaxSteps={flashedAnchor === "max-steps"}
            />
          ) : null}
        </div>

        {draft.role === "primary" ? (
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
        ) : draft.role === "subagent" ? (
          <AgentToolsGrid
            draft={draft}
            bindableTools={bindableToolsSubagent}
            mcpServers={mcpList}
            saveBlocked={saveBlocked}
            onBindingChange={updateBinding}
            note={t("settings.agents.subagentsNote")}
          />
        ) : null}
      </div>
      {dialog}
    </SettingsPageShell>
  );
}

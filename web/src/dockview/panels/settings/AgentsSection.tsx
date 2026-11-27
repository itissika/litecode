import { useEffect, useMemo, useState } from "react";
import { ArrowClockwise } from "@phosphor-icons/react";
import { useTranslation } from "react-i18next";

import {
  presetToolIds,
  settingsToolSwitches,
  applyToolEnabled,
  type AgentProfile,
  type AgentRole,
  type AgentToolBinding,
  type CatalogModelDto,
} from "../../../api/settings";
import type { ModelInfo } from "../../../api/types";
import { useSettingsStore } from "../../../stores/settingsStore";
import { mergeLayeredMcp } from "../../../stores/settingsDocuments";
import { ModelSwitcher } from "../../../components/ModelSwitcher";
import { Select } from "../../../components/ui/Select";
import { AgentTypeIcon, agentColor } from "../../../components/agentIdentity";
import {
  AgentToolsGrid,
  bindingFor,
} from "../../../components/agent/AgentToolsGrid";
import {
  FieldLabel,
  SettingsPageShell,
  useSettingsSaveBlocked,
} from "./shared";
import {
  flushRegisteredSettings,
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

// Bindings for tools not in this workspace catalog stay on the profile so a PUT
// does not wipe other-workspace MCP/custom rows. The grid only shows catalog ids.
/**
 * Allowed-subagent chip: a primary reads as a filled chip, a subagent as an
 * empty hairline frame.
 */
function agentChipClass(role: AgentRole): string {
  return role === "primary"
    ? "tag tag-neutral tag-sm tag-soft"
    : "tag tag-neutral tag-sm";
}

function agentPersistPayload(draft: AgentProfile): AgentProfile {
  return {
    ...draft,
    tools: { ...draft.tools },
  };
}

/**
 * Settings → Agents.
 *
 * Every agent here is a LiteCode spec: identity, prompt, and core tools are
 * system-tuned and read-only (the backend drops those on write). The page keeps
 * only the two things the user owns — the model and the non-core tool cards
 * (search, LSP, web, custom, MCP).
 */
export function AgentsSection() {
  const { t } = useTranslation();
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
  const saveAgent = useSettingsStore((s) => s.saveAgent);
  const refreshAgents = useSettingsStore((s) => s.refreshAgents);

  // Picker order only: primary agents first, subagents after (store order
  // inside a group). The role itself is spec-owned and not user-sortable.
  const agentOptionIds = useMemo(() => {
    const rank = (id: string) => (agents[id]?.role === "primary" ? 0 : 1);
    return [...agentIds].sort((a, b) => rank(a) - rank(b));
  }, [agentIds, agents]);

  const profile = agents[selectedAgentId];
  const [draft, setDraft] = useState<AgentProfile | null>(null);

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
    if (!profile) return;
    if (!shouldHydrateDraftFromStore(persistStatus)) return;
    setDraft({
      ...profile,
      allowed_subagents: profile.allowed_subagents ?? [],
      tools: { ...profile.tools },
    });
  }, [profile, selectedAgentId, persistStatus]);

  useSettingsPersist<AgentProfile | null, AgentProfile>(draft, {
    enabled: draft != null,
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

  const updateBinding = (toolId: string, patch: Partial<AgentToolBinding>) => {
    if (!draft) return;
    // Locked cards are spec-owned: a stray click must not write a row, whether
    // it came from the toggle or a preset button.
    const entry = switchTools.find((tool) => tool.id === toolId);
    if (entry?.locked) return;
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

  if (!draft) {
    return (
      <SettingsPageShell title={t('settings.agents.title')}>
        <p className="text-sm text-(--_dk-text-disabled)">{t("settings.agents.loading")}</p>
      </SettingsPageShell>
    );
  }

  const allowedSubagents = draft.allowed_subagents ?? [];

  return (
    <SettingsPageShell
      title={t('settings.agents.title')}
      actions={
        <button
          type="button"
          onClick={() => void refreshAgents()}
          className="btn btn-icon"
          aria-label={t("settings.agents.refresh")}
          title={t("settings.agents.refresh")}
        >
          <ArrowClockwise size={16} />
        </button>
      }
    >
      <div className="space-y-6">
        <div className="space-y-3">
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
                  options={agentOptionIds.map((id) => {
                    const role = agents[id]?.role ?? "primary";
                    return {
                      value: id,
                      label: (
                        <span className="flex items-center gap-1.5">
                          <AgentTypeIcon role={role} color={agentColor(id)} />
                          <span>{id}</span>
                          {/* The role trails the name at a smaller size, as in
                              `name -- type`. */}
                          <span className="flex shrink-0 items-center gap-1 text-[0.65rem]">
                            <span className="text-(--_dk-text-disabled)">
                              --
                            </span>
                            <span className="text-(--_dk-text-muted)">
                              {role === "primary"
                                ? t("settings.agents.primary")
                                : t("settings.agents.subagentRole")}
                            </span>
                          </span>
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
            </div>
            {draft.description || allowedSubagents.length > 0 ? (
              <div className="flex min-w-0 flex-col gap-1 text-xs text-(--_dk-text-muted)">
                {draft.description ? (
                  <span data-agent-description className="min-w-0 truncate">
                    {draft.description}
                  </span>
                ) : null}
                {allowedSubagents.length > 0 ? (
                  <span className="flex flex-wrap items-center gap-1">
                    <span>{t("settings.agents.allowedSubagents")}:</span>
                    {allowedSubagents.map((id) => {
                      const role = agents[id]?.role ?? "subagent";
                      return (
                        <span key={id} className={agentChipClass(role)}>
                          <AgentTypeIcon role={role} />
                          {id}
                        </span>
                      );
                    })}
                  </span>
                ) : null}
              </div>
            ) : null}
          </div>
        </div>

        <AgentToolsGrid
          draft={draft}
          bindableTools={
            draft.role === "primary"
              ? bindableToolsPrimary
              : bindableToolsSubagent
          }
          mcpServers={mcpList}
          saveBlocked={saveBlocked}
          onBindingChange={updateBinding}
        />
      </div>
    </SettingsPageShell>
  );
}

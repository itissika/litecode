import { create } from "zustand";

import { attachSiblingStores } from "./connectionStore";

import {
  getAgent,
  getAvailableTools,
  getAdapters,
  getLog,
  getExcludes,
  getModels,
  getProviders,
  getWebSearch,
  getSettingsSummary,
  loadSettingsAgentIds,
  deleteAgent,
  putAgent,
  applyAgentToolPreset,
  putLog,
  putExcludes,
  putModels,
  putProviders,
  putWebSearch,
  getCustomTools,
  putCustomTool,
  deleteCustomTool,
  getMcpServers,
  putMcpServer,
  deleteMcpServer,
  startMcpServer as requestMcpStart,
  restartMcpServer as requestMcpRestart,
  stopMcpServer as requestMcpStop,
  SettingsApiError,
  withSyncedToolSeries,
  type AdapterDescriptor,
  type AgentProfile,
  type AvailableTool,
  type CustomToolDefinition,
  type LayeredList,
  type McpProbeResult,
  type McpServerDefinition,
  type McpServerItem,
  type LogSettings,
  type WorkspaceExcludes,
  type WorkspaceExcludesLists,
  type ModelDefinition,
  type ProviderDefinition,
  type ProviderView,
  type WebSearchView,
  type SettingsSummary,
  type ToolPreset,
  type ToolScope,
} from "../api/settings";
import type { SettingsChanged } from "../api/types";
import { useSessionStore } from "./sessionStore";
import { useTurnStore } from "./turnStore";
import { useToastStore } from "./toastStore";
import {
  flushRegisteredSettings,
  SETTINGS_PERSIST_ERROR_CHANNEL,
  type PersistStatus,
} from "../dockview/panels/settings/persist";

export type { PersistStatus };

export type SettingsSection =
  | "connection"
  | "models"
  | "engines"
  | "custom-tools"
  | "mcp"
  | "agents"
  | "files"
  | "advanced";

export function sectionNeedsSkeleton(
  section: SettingsSection,
  state: {
    providers: Record<string, ProviderView> | null;
    models: Record<string, ModelDefinition> | null;
    availableTools: AvailableTool[] | null;
    customTools: LayeredList<CustomToolDefinition> | null;
    mcpServers: LayeredList<McpServerItem> | null;
    log: LogSettings | null;
    websearch: WebSearchView | null;
    excludes: WorkspaceExcludes | null;
  },
): boolean {
  switch (section) {
    case "connection":
      return state.providers === null;
    case "models":
      return state.models === null || state.providers === null;
    case "agents":
      return state.availableTools === null || state.models === null;
    case "custom-tools":
      return state.customTools === null;
    case "mcp":
      return state.mcpServers === null;
    case "files":
      return state.excludes === null;
    case "advanced":
      return state.log === null || state.websearch === null;
    case "engines":
      return false;
  }
}

interface SettingsStoreState {
  open: boolean;
  section: SettingsSection;
  revision: number;
  summary: SettingsSummary | null;
  adapters: AdapterDescriptor[];
  providers: Record<string, ProviderView> | null;
  websearch: WebSearchView | null;
  models: Record<string, ModelDefinition> | null;
  availableTools: AvailableTool[] | null;
  customTools: LayeredList<CustomToolDefinition> | null;
  mcpServers: LayeredList<McpServerItem> | null;
  agentIds: string[];
  selectedAgentId: string;
  agents: Record<string, AgentProfile>;
  log: LogSettings | null;
  excludes: WorkspaceExcludes | null;
  loadError: string | null;
  persistStatus: PersistStatus;
  loadedRevisionBySection: Partial<Record<SettingsSection, number>>;
}

interface SettingsStore extends SettingsStoreState {
  openSettings: (section?: SettingsSection) => void;
  closeSettings: () => Promise<void>;
  setSection: (section: SettingsSection) => Promise<void>;
  setPersistStatus: (status: PersistStatus) => void;
  setRevision: (revision: number) => void;
  onRemoteSettingsChanged: (event: SettingsChanged) => void;
  notifySetupIfNeeded: () => Promise<void>;
  ensureSectionLoaded: (section: SettingsSection, force?: boolean) => Promise<void>;
  refreshAgents: () => Promise<void>;
  setSelectedAgentId: (id: string) => void;
  saveProviders: (providers: Record<string, ProviderDefinition>) => Promise<void>;
  saveWebSearch: (body: { search_endpoint?: string }) => Promise<void>;
  saveModels: (models: Record<string, ModelDefinition>) => Promise<void>;
  saveCustomTool: (id: string, def: CustomToolDefinition, scope?: ToolScope) => Promise<void>;
  removeCustomTool: (id: string, scope?: ToolScope) => Promise<void>;
  saveMcpServer: (id: string, def: McpServerDefinition, scope?: ToolScope) => Promise<void>;
  removeMcpServer: (id: string, scope?: ToolScope) => Promise<void>;
  startMcpServer: (id: string, def: McpServerDefinition, scope?: ToolScope) => Promise<McpProbeResult>;
  restartMcpServer: (id: string, def: McpServerDefinition, scope?: ToolScope) => Promise<McpProbeResult>;
  stopMcpServer: (id: string, scope?: ToolScope) => Promise<McpProbeResult>;
  saveAgent: (id: string, profile: AgentProfile) => Promise<void>;
  applyAgentToolPreset: (id: string, preset: ToolPreset) => Promise<void>;
  createAgent: (id: string, profile: AgentProfile) => Promise<void>;
  removeAgent: (id: string) => Promise<void>;
  saveLog: (level: string | null) => Promise<void>;
  saveExcludes: (body: WorkspaceExcludesLists) => Promise<void>;
  isSaveBlocked: () => boolean;
}

function turnInProgress(): boolean {
  // Check if any session has a running or cancelling turn.
  const { byId } = useTurnStore.getState();
  for (const slice of byId.values()) {
    if (slice.runState === "running" || slice.runState === "cancelling") {
      return true;
    }
  }
  return false;
}

function handleSaveError(err: unknown): void {
  if (err instanceof SettingsApiError && err.isTurnBlocked) {
    useToastStore
      .getState()
      .showToast(
        "Cannot save settings while an agent turn is in progress",
        "error",
        5000,
        SETTINGS_PERSIST_ERROR_CHANNEL,
      );
    return;
  }
  const message = err instanceof Error ? err.message : "Settings save failed";
  if (message === "turn_in_progress") {
    useToastStore
      .getState()
      .showToast(
        "Cannot change engines while an agent turn is in progress",
        "error",
        5000,
        SETTINGS_PERSIST_ERROR_CHANNEL,
      );
    return;
  }
  useToastStore.getState().showToast(message, "error", 5000, SETTINGS_PERSIST_ERROR_CHANNEL);
}

/** Toast whenever setup is incomplete — every time, no hard gate. */
function toastSetupGuidance(summary: SettingsSummary | null | undefined): void {
  const guidance = summary?.setup_guidance?.trim();
  if (!guidance) return;
  useToastStore.getState().showToast(guidance, "info", 12000);
}

/**
 * Corner toast for LLM / AI-setup failures (not the bell).
 * Prefers backend `setup_guidance`; otherwise uses the RPC/turn message or a
 * default that names default (primary) and compaction (hidden).
 */
export function toastLlmConfigFailure(fallback?: string): void {
  const guidance = useSettingsStore.getState().summary?.setup_guidance?.trim();
  const message =
    guidance ||
    fallback?.trim() ||
    "AI setup incomplete — assign models to default (primary) and compaction (hidden) in Settings → Agents. Agent runs will fail until this is fixed.";
  useToastStore.getState().showToast(message, "info", 12000);
}


export const useSettingsStore = create<SettingsStore>((set, get) => ({
  open: false,
  section: "connection",
  revision: 0,
  summary: null,
  adapters: [],
  providers: null,
  websearch: null,
  models: null,
  availableTools: null,
  customTools: null,
  mcpServers: null,
  agentIds: ["default"],
  selectedAgentId: "default",
  agents: {},
  log: null,
  excludes: null,
  loadError: null,
  persistStatus: "idle",
  loadedRevisionBySection: {},

  openSettings: (section = "connection") => {
    set({ open: true, section, persistStatus: "idle", loadError: null });
    void get().ensureSectionLoaded(section);
  },

  closeSettings: async () => {
    await flushRegisteredSettings();
    set({ open: false, persistStatus: "idle" });
  },

  setSection: async (section) => {
    if (get().section === section) return;
    await flushRegisteredSettings();
    set({ section, persistStatus: "idle", loadError: null });
    void get().ensureSectionLoaded(section);
  },

  setPersistStatus: (persistStatus) => {
    if (get().persistStatus === persistStatus) return;
    set({ persistStatus });
  },

  setRevision: (revision) => {
    const { revision: current, open, section } = get();
    set({ revision });
    if (open && revision > current) {
      void get().ensureSectionLoaded(section, true);
    }
  },

  onRemoteSettingsChanged: (event) => {
    set({ summary: event.summary, revision: event.revision });
    if (event.summary.restart_required) {
      useToastStore
        .getState()
        .showToast("Settings changed — restart the server to apply", "info");
    } else if (event.summary.setup_guidance) {
      toastSetupGuidance(event.summary);
    } else if (event.summary.effective_next_turn && !get().open) {
      useToastStore
        .getState()
        .showToast("Settings changed — effective next turn", "success");
    }
    void useSessionStore.getState().refreshAvailableModels();
    if (get().open) {
      const status = get().persistStatus;
      if (status === "pending" || status === "saving") {
        return;
      }
      void get().ensureSectionLoaded(get().section, true);
    }
  },

  notifySetupIfNeeded: async () => {
    try {
      const summary = await getSettingsSummary();
      set({ summary, revision: summary.revision });
      toastSetupGuidance(summary);
    } catch {
      // Connect path already surfaces transport errors elsewhere.
    }
  },

  isSaveBlocked: () => turnInProgress(),

  ensureSectionLoaded: async (section, force = false) => {
    if (section === "engines") return;
    const state = get();
    if (
      !force &&
      state.loadedRevisionBySection[section] === state.revision &&
      !sectionNeedsSkeleton(section, state)
    ) {
      return;
    }
    try {
      const markFresh = (revision: number) => ({
        loadedRevisionBySection: {
          ...get().loadedRevisionBySection,
          [section]: revision,
        },
        loadError: null,
      });
      if (section === "connection") {
        const [summary, adapters, providers] = await Promise.all([
          getSettingsSummary(),
          getAdapters(),
          getProviders(),
        ]);
        set({
          summary,
          revision: summary.revision,
          adapters,
          providers,
          ...markFresh(summary.revision),
        });
        return;
      }
      if (section === "models") {
        const [adapters, providers, models] = await Promise.all([
          getAdapters(),
          getProviders(),
          getModels(),
        ]);
        set({ adapters, providers, models, ...markFresh(get().revision) });
        void get().refreshAgents();
        return;
      }
      if (section === "agents") {
        const [models, availableTools, mcpServers] = await Promise.all([
          getModels(),
          getAvailableTools(),
          getMcpServers(),
        ]);
        set({ models, availableTools, mcpServers });
        await get().refreshAgents();
        set(markFresh(get().revision));
        return;
      }
      if (section === "custom-tools") {
        const customTools = await getCustomTools();
        set({ customTools, ...markFresh(get().revision) });
        return;
      }
      if (section === "mcp") {
        const mcpServers = await getMcpServers();
        set({ mcpServers, ...markFresh(get().revision) });
        return;
      }
      if (section === "files") {
        const excludes = await getExcludes();
        set({ excludes, ...markFresh(get().revision) });
        return;
      }
      if (section === "advanced") {
        const [log, websearch] = await Promise.all([getLog(), getWebSearch()]);
        set({ log, websearch, ...markFresh(get().revision) });
      }
    } catch (err) {
      const message = err instanceof Error ? err.message : "Failed to load settings";
      set({ loadError: message });
      useToastStore.getState().showToast(message, "error");
    }
  },

  refreshAgents: async () => {
    const agentIds = await loadSettingsAgentIds();
    const agents: Record<string, AgentProfile> = {};
    await Promise.all(
      agentIds.map(async (id) => {
        agents[id] = await getAgent(id);
      }),
    );
    set({
      agentIds,
      agents,
      selectedAgentId: agentIds.includes(get().selectedAgentId)
        ? get().selectedAgentId
        : agentIds[0] ?? "default",
    });
  },

  setSelectedAgentId: (id) => set({ selectedAgentId: id }),

  saveProviders: async (providers) => {
    if (turnInProgress()) {
      const err = new SettingsApiError(409, "turn_in_progress");
      handleSaveError(err);
      throw err;
    }
    try {
      const { revision } = await putProviders(providers);
      const next = await getProviders();
      set({ revision, providers: next });
    } catch (err) {
      handleSaveError(err);
      throw err;
    }
  },

  saveWebSearch: async (body) => {
    if (turnInProgress()) {
      const err = new SettingsApiError(409, "turn_in_progress");
      handleSaveError(err);
      throw err;
    }
    try {
      const { revision } = await putWebSearch(body);
      const websearch = await getWebSearch();
      set({ revision, websearch });
    } catch (err) {
      handleSaveError(err);
      throw err;
    }
  },

  saveModels: async (models) => {
    if (turnInProgress()) {
      const err = new SettingsApiError(409, "turn_in_progress");
      handleSaveError(err);
      throw err;
    }
    try {
      const { revision } = await putModels(models);
      set({ revision, models });
      void useSessionStore.getState().refreshAvailableModels();
    } catch (err) {
      handleSaveError(err);
      throw err;
    }
  },

  saveCustomTool: async (id, def, scope = "global") => {
    if (turnInProgress()) {
      const err = new SettingsApiError(409, "turn_in_progress");
      handleSaveError(err);
      throw err;
    }
    try {
      const { revision } = await putCustomTool(id, def, scope);
      const [customTools, availableTools] = await Promise.all([
        getCustomTools(),
        getAvailableTools(),
      ]);
      set({ revision, customTools, availableTools });
    } catch (err) {
      handleSaveError(err);
      throw err;
    }
  },

  removeCustomTool: async (id, scope = "global") => {
    if (turnInProgress()) {
      const err = new SettingsApiError(409, "turn_in_progress");
      handleSaveError(err);
      throw err;
    }
    try {
      const { revision } = await deleteCustomTool(id, scope);
      const [customTools, availableTools] = await Promise.all([
        getCustomTools(),
        getAvailableTools(),
      ]);
      await get().refreshAgents();
      set({ revision, customTools, availableTools });
    } catch (err) {
      handleSaveError(err);
      throw err;
    }
  },

  saveMcpServer: async (id, def, scope = "global") => {
    if (turnInProgress()) {
      const err = new SettingsApiError(409, "turn_in_progress");
      handleSaveError(err);
      throw err;
    }
    try {
      const { revision } = await putMcpServer(id, def, scope);
      const [mcpServers, availableTools] = await Promise.all([
        getMcpServers(),
        getAvailableTools(),
      ]);
      set({ revision, mcpServers, availableTools });
    } catch (err) {
      handleSaveError(err);
      throw err;
    }
  },

  removeMcpServer: async (id, scope = "global") => {
    if (turnInProgress()) {
      const err = new SettingsApiError(409, "turn_in_progress");
      handleSaveError(err);
      throw err;
    }
    try {
      const { revision } = await deleteMcpServer(id, scope);
      const [mcpServers, availableTools] = await Promise.all([
        getMcpServers(),
        getAvailableTools(),
      ]);
      await get().refreshAgents();
      set({ revision, mcpServers, availableTools });
    } catch (err) {
      handleSaveError(err);
      throw err;
    }
  },

  startMcpServer: async (id, def, scope = "global") => {
    const result = await requestMcpStart(id, def, scope);
    const mcpServers = await getMcpServers();
    set({ mcpServers });
    return result;
  },

  restartMcpServer: async (id, def, scope = "global") => {
    const result = await requestMcpRestart(id, def, scope);
    const mcpServers = await getMcpServers();
    set({ mcpServers });
    return result;
  },

  stopMcpServer: async (id, scope = "global") => {
    const result = await requestMcpStop(id, scope);
    const mcpServers = await getMcpServers();
    set({ mcpServers });
    return result;
  },

  saveAgent: async (id, profile) => {
    if (turnInProgress()) {
      const err = new SettingsApiError(409, "turn_in_progress");
      handleSaveError(err);
      throw err;
    }
    try {
      const synced = withSyncedToolSeries(profile);
      const { revision } = await putAgent(id, synced);
      set((s) => ({
        revision,
        agents: { ...s.agents, [id]: synced },
      }));
    } catch (err) {
      handleSaveError(err);
      throw err;
    }
  },

  applyAgentToolPreset: async (id, preset) => {
    if (turnInProgress()) {
      const err = new SettingsApiError(409, "turn_in_progress");
      handleSaveError(err);
      throw err;
    }
    try {
      const { revision } = await applyAgentToolPreset(id, preset);
      const profile = await getAgent(id);
      set((s) => ({
        revision,
        agents: { ...s.agents, [id]: profile },
      }));
    } catch (err) {
      handleSaveError(err);
      throw err;
    }
  },

  createAgent: async (id, profile) => {
    await get().saveAgent(id, profile);
    await get().refreshAgents();
    set({ selectedAgentId: id });
  },

  removeAgent: async (id) => {
    if (turnInProgress()) {
      const err = new SettingsApiError(409, "turn_in_progress");
      handleSaveError(err);
      throw err;
    }
    try {
      const { revision } = await deleteAgent(id);
      const agentIds = await loadSettingsAgentIds();
      const agents: Record<string, AgentProfile> = {};
      await Promise.all(
        agentIds.map(async (agentId) => {
          agents[agentId] = await getAgent(agentId);
        }),
      );
      set({
        revision,
        agentIds,
        agents,
        selectedAgentId: agentIds.includes(get().selectedAgentId)
          ? get().selectedAgentId
          : agentIds[0] ?? "default",
      });
    } catch (err) {
      handleSaveError(err);
      throw err;
    }
  },

  saveLog: async (level) => {
    if (turnInProgress()) {
      const err = new SettingsApiError(409, "turn_in_progress");
      handleSaveError(err);
      throw err;
    }
    try {
      const { revision } = await putLog(level);
      set({ revision, log: { level } });
    } catch (err) {
      handleSaveError(err);
      throw err;
    }
  },

  saveExcludes: async (body) => {
    if (turnInProgress()) {
      const err = new SettingsApiError(409, "turn_in_progress");
      handleSaveError(err);
      throw err;
    }
    try {
      const excludes = await putExcludes(body);
      set({ excludes });
    } catch (err) {
      handleSaveError(err);
      throw err;
    }
  },
}));

attachSiblingStores({ settings: useSettingsStore });

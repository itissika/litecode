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
  type EngineStatus,
} from "../api/settings";
import { getEnginesDetail, type EnginesDetail, type LspInstanceStatusView } from "../api/workspace";
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

// Catalog warmup poll: exponential backoff, then a quiet keep-alive at the
// max delay. Engines (ORT / LSP) can stay `warming` for minutes; that is
// visible in Settings → Engines and must not spam error toasts (FE-03).
const WARMUP_BASE_MS = 500;
const WARMUP_MAX_DELAY_MS = 8000;
const WARMUP_MAX_ATTEMPTS = 8;
let warmupAttempts = 0;
let catalogPollTimer: ReturnType<typeof setTimeout> | null = null;
let catalogFetchErrorToasted = false;

/** Exponential backoff delay (ms) for the `attempt`-th catalog poll (1-based). */
export function catalogPollDelayMs(attempt: number): number {
  const n = Math.max(0, attempt - 1);
  return Math.min(WARMUP_BASE_MS * 2 ** n, WARMUP_MAX_DELAY_MS);
}

function clearCatalogPollTimer(): void {
  if (catalogPollTimer !== null) {
    clearTimeout(catalogPollTimer);
    catalogPollTimer = null;
  }
}

/** Test hook: module poll state is not in the zustand store. */
export function resetCatalogPollState(): void {
  warmupAttempts = 0;
  catalogFetchErrorToasted = false;
  clearCatalogPollTimer();
}

function markCatalogSettled(): void {
  warmupAttempts = 0;
  catalogFetchErrorToasted = false;
  clearCatalogPollTimer();
}

/** Coalesce overlapping callers (hello + settings/changed + Engines 1s refresh). */
function scheduleCatalogPoll(error?: unknown): void {
  if (catalogPollTimer !== null) {
    return;
  }
  warmupAttempts += 1;
  if (warmupAttempts > WARMUP_MAX_ATTEMPTS) {
    warmupAttempts = WARMUP_MAX_ATTEMPTS;
    if (error instanceof Error && !catalogFetchErrorToasted) {
      catalogFetchErrorToasted = true;
      useToastStore.getState().showToast(error.message, "error");
    }
  }
  const delay = catalogPollDelayMs(warmupAttempts);
  catalogPollTimer = setTimeout(() => {
    catalogPollTimer = null;
    void useSettingsStore.getState().ensureCatalogLoaded();
  }, delay);
}

interface SettingsStoreState {
  open: boolean;
  section: SettingsSection;
  revision: number;
  summary: SettingsSummary | null;
  adapters: AdapterDescriptor[];
  providers: Record<string, ProviderView> | null;
  /** @deprecated first provider convenience */
  provider: ProviderView | null;
  websearch: WebSearchView | null;
  models: Record<string, ModelDefinition> | null;
  availableTools: AvailableTool[] | null;
  customTools: LayeredList<CustomToolDefinition> | null;
  mcpServers: LayeredList<McpServerItem> | null;
  engineStatuses: Record<string, EngineStatus>;
  /** Live language-server instances from engines detail (Running = server ready). */
  lspServers: LspInstanceStatusView[];
  agentIds: string[];
  selectedAgentId: string;
  agents: Record<string, AgentProfile>;
  log: LogSettings | null;
  excludes: WorkspaceExcludes | null;
  loading: boolean;
  saving: boolean;
  loadError: string | null;
  restartRequired: boolean;
  persistStatus: PersistStatus;
}

interface SettingsStore extends SettingsStoreState {
  openSettings: (section?: SettingsSection) => void;
  closeSettings: () => Promise<void>;
  setSection: (section: SettingsSection) => Promise<void>;
  setPersistStatus: (status: PersistStatus) => void;
  setRevision: (revision: number) => void;
  onRemoteSettingsChanged: (event: SettingsChanged) => void;
  /** Load engine statuses (needed for Editor LSP gate). */
  ensureCatalogLoaded: () => Promise<void>;
  /** Refresh only engineStatuses after Engine panel start/stop/clear. */
  refreshEngineStatuses: () => Promise<void>;
  /** Fetch summary and toast if AI setup is incomplete. */
  notifySetupIfNeeded: () => Promise<void>;
  refresh: () => Promise<void>;
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


function enginesFromDetail(detail: EnginesDetail): Record<string, EngineStatus> {
  return {
    lsp: {
      desired: detail.lsp.desired,
      state: detail.lsp.state,
      error: detail.lsp.error,
    },
    code_search: {
      desired: detail.retrieval.desired,
      state: detail.retrieval.state,
      error: detail.retrieval.error,
    },
  };
}

function lspServersFromDetail(detail: EnginesDetail): LspInstanceStatusView[] {
  return detail.lsp.servers ?? [];
}

export const useSettingsStore = create<SettingsStore>((set, get) => ({
  open: false,
  section: "connection",
  revision: 0,
  summary: null,
  adapters: [],
  providers: null,
  provider: null,
  websearch: null,
  models: null,
  availableTools: null,
  customTools: null,
  mcpServers: null,
  engineStatuses: {},
  lspServers: [],
  agentIds: ["default"],
  selectedAgentId: "default",
  agents: {},
  log: null,
  excludes: null,
  loading: false,
  saving: false,
  loadError: null,
  restartRequired: false,
  persistStatus: "idle",

  openSettings: (section = "connection") => {
    set({ open: true, section, persistStatus: "idle" });
    void get().refresh();
  },

  closeSettings: async () => {
    await flushRegisteredSettings();
    set({ open: false, persistStatus: "idle" });
  },

  setSection: async (section) => {
    if (get().section === section) return;
    await flushRegisteredSettings();
    set({ section, persistStatus: "idle" });
  },

  setPersistStatus: (persistStatus) => {
    if (get().persistStatus === persistStatus) return;
    set({ persistStatus });
  },

  setRevision: (revision) => {
    const { revision: current, open } = get();
    if (revision > current) {
      set({ revision });
      if (open) {
        void get().refresh();
      }
    } else {
      set({ revision });
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
    void get().ensureCatalogLoaded();
    void useSessionStore.getState().refreshAvailableModels();
    if (get().open) {
      const status = get().persistStatus;
      // Own-write echo: persist already applied the payload. A full refresh
      // would flip `loading` and remount the section onto a skeleton.
      if (status === "pending" || status === "saving" || status === "saved") {
        return;
      }
      void get().refresh();
    }
  },

  ensureCatalogLoaded: async () => {
    try {
      const detail = await getEnginesDetail();
      const engineStatuses = enginesFromDetail(detail);
      set({ engineStatuses, lspServers: lspServersFromDetail(detail) });
      const warming = Object.values(engineStatuses).some(
        (engine) => engine?.state === "warming",
      );
      if (warming) {
        scheduleCatalogPoll();
      } else {
        markCatalogSettled();
      }
    } catch (err) {
      scheduleCatalogPoll(err);
    }
  },

  refreshEngineStatuses: async () => {
    try {
      const detail = await getEnginesDetail();
      const engineStatuses = enginesFromDetail(detail);
      set({ engineStatuses, lspServers: lspServersFromDetail(detail) });
      const warming = Object.values(engineStatuses).some(
        (engine) => engine?.state === "warming",
      );
      if (warming) {
        scheduleCatalogPoll();
      }
    } catch (err) {
      console.error("engine status refresh failed", err);
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

  refresh: async () => {
    const firstLoad = !get().adapters.length && get().providers === null;
    if (firstLoad) {
      set({ loading: true, loadError: null });
    }
    try {
      // Config only. Engine live-state (LSP probes, index, warmup) lives on
      // ensureCatalogLoaded / EnginesSection — waiting for it here blocks every
      // settings page, including Provider.
      const [
        summary,
        adapters,
        providers,
        websearch,
        models,
        availableTools,
        customTools,
        mcpServers,
        log,
        excludes,
        agentBundle,
      ] = await Promise.all([
        getSettingsSummary(),
        getAdapters(),
        getProviders(),
        getWebSearch(),
        getModels(),
        getAvailableTools(),
        getCustomTools(),
        getMcpServers(),
        getLog(),
        getExcludes(),
        (async () => {
          const agentIds = await loadSettingsAgentIds();
          const agents: Record<string, AgentProfile> = {};
          await Promise.all(
            agentIds.map(async (id) => {
              agents[id] = await getAgent(id);
            }),
          );
          return { agentIds, agents };
        })(),
      ]);
      const { agentIds, agents } = agentBundle;
      const firstProvider = Object.values(providers)[0] ?? null;
      set({
        summary,
        revision: summary.revision,
        adapters,
        providers,
        provider: firstProvider,
        websearch,
        models,
        availableTools,
        customTools,
        mcpServers,
        agentIds,
        agents,
        selectedAgentId: agentIds.includes(get().selectedAgentId)
          ? get().selectedAgentId
          : agentIds[0] ?? "default",
        log,
        excludes,
        loading: false,
      });
    } catch (err) {
      const message = err instanceof Error ? err.message : "Failed to load settings";
      set({ loading: false, loadError: message });
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
    set({ saving: true });
    try {
      const { revision } = await putProviders(providers);
      const next = await getProviders();
      set({
        revision,
        providers: next,
        provider: Object.values(next)[0] ?? null,
        saving: false,
      });
    } catch (err) {
      set({ saving: false });
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
    set({ saving: true });
    try {
      const { revision } = await putWebSearch(body);
      const websearch = await getWebSearch();
      set({ revision, websearch, saving: false });
    } catch (err) {
      set({ saving: false });
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
    set({ saving: true });
    try {
      const { revision } = await putModels(models);
      set({ revision, models, saving: false });
      void useSessionStore.getState().refreshAvailableModels();
    } catch (err) {
      set({ saving: false });
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
    set({ saving: true });
    try {
      const { revision } = await putCustomTool(id, def, scope);
      const [customTools, availableTools] = await Promise.all([
        getCustomTools(),
        getAvailableTools(),
      ]);
      set({
        revision,
        customTools,
        availableTools,
        saving: false,
      });
    } catch (err) {
      set({ saving: false });
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
    set({ saving: true });
    try {
      const { revision } = await deleteCustomTool(id, scope);
      const [customTools, availableTools] = await Promise.all([
        getCustomTools(),
        getAvailableTools(),
      ]);
      await get().refreshAgents();
      set({
        revision,
        customTools,
        availableTools,
        saving: false,
      });
    } catch (err) {
      set({ saving: false });
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
    set({ saving: true });
    try {
      const { revision } = await putMcpServer(id, def, scope);
      const [mcpServers, availableTools] = await Promise.all([
        getMcpServers(),
        getAvailableTools(),
      ]);
      set({
        revision,
        mcpServers,
        availableTools,
        saving: false,
      });
    } catch (err) {
      set({ saving: false });
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
    set({ saving: true });
    try {
      const { revision } = await deleteMcpServer(id, scope);
      const [mcpServers, availableTools] = await Promise.all([
        getMcpServers(),
        getAvailableTools(),
      ]);
      await get().refreshAgents();
      set({
        revision,
        mcpServers,
        availableTools,
        saving: false,
      });
    } catch (err) {
      set({ saving: false });
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
    set({ saving: true });
    try {
      const synced = withSyncedToolSeries(profile);
      const { revision } = await putAgent(id, synced);
      set((s) => ({
        revision,
        agents: { ...s.agents, [id]: synced },
        saving: false,
      }));
    } catch (err) {
      set({ saving: false });
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
    set({ saving: true });
    try {
      const { revision } = await applyAgentToolPreset(id, preset);
      const profile = await getAgent(id);
      set((s) => ({
        revision,
        agents: { ...s.agents, [id]: profile },
        saving: false,
      }));
    } catch (err) {
      set({ saving: false });
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
    set({ saving: true });
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
        saving: false,
      });
    } catch (err) {
      set({ saving: false });
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
    set({ saving: true });
    try {
      const { revision } = await putLog(level);
      set({ revision, log: { level }, saving: false });
    } catch (err) {
      set({ saving: false });
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
    set({ saving: true });
    try {
      const excludes = await putExcludes(body);
      set({ excludes, saving: false });
    } catch (err) {
      set({ saving: false });
      handleSaveError(err);
      throw err;
    }
  },
}));

attachSiblingStores({ settings: useSettingsStore });

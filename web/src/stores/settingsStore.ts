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
  type ToolScope,
} from "../api/settings";
import type { SettingsChanged } from "../api/types";
import type { WorkspaceChangeKind } from "../api/workspace";
import { useSessionStore } from "./sessionStore";
import { anyTurnRunning } from "./turnStore";
import { useToastStore } from "./toastStore";
import {
  flushRegisteredSettings,
  SETTINGS_PERSIST_ERROR_CHANNEL,
  type PersistStatus,
} from "../lib/settingsPersist";
import {
  documentIsFresh,
  EXCLUDES_CLOCK,
  isWorkspaceCustomToolsPath,
  isWorkspaceExcludesPath,
  isWorkspaceMcpPath,
  mergeLayeredMcp,
  SECTION_DOCUMENTS,
  sectionNeedsSkeleton,
  splitMcpListing,
  type LayeredMcpRuntime,
  type McpDefItem,
  type SettingsDocClock,
  type SettingsDocument,
  type SettingsSection,
} from "./settingsDocuments";

export type { PersistStatus, SettingsSection };
export { sectionNeedsSkeleton };

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
  mcpDefs: LayeredList<McpDefItem> | null;
  mcpRuntime: LayeredMcpRuntime | null;
  agentIds: string[];
  selectedAgentId: string;
  agents: Record<string, AgentProfile>;
  log: LogSettings | null;
  excludes: WorkspaceExcludes | null;
  loadError: string | null;
  persistStatus: PersistStatus;
  docClock: SettingsDocClock;
}

interface SettingsStore extends SettingsStoreState {
  openSettings: (section?: SettingsSection) => void;
  closeSettings: () => Promise<void>;
  setSection: (section: SettingsSection) => Promise<void>;
  setPersistStatus: (status: PersistStatus) => void;
  setRevision: (revision: number) => void;
  onRemoteSettingsChanged: (event: SettingsChanged) => void;
  handleWorkspaceChange: (paths: string[], kind: WorkspaceChangeKind) => void;
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
  createAgent: (id: string, profile: AgentProfile) => Promise<void>;
  removeAgent: (id: string) => Promise<void>;
  saveLog: (level: string | null) => Promise<void>;
  saveExcludes: (body: WorkspaceExcludesLists) => Promise<void>;
}

let loadFlight = 0;

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
        "Cannot save settings while an agent turn is in progress",
        "error",
        5000,
        SETTINGS_PERSIST_ERROR_CHANNEL,
      );
    return;
  }
  useToastStore.getState().showToast(message, "error", 5000, SETTINGS_PERSIST_ERROR_CHANNEL);
}

async function withTurnGuard<T>(fn: () => Promise<T>): Promise<T> {
  if (anyTurnRunning()) {
    const err = new SettingsApiError(409, "turn_in_progress");
    handleSaveError(err);
    throw err;
  }
  try {
    return await fn();
  } catch (err) {
    handleSaveError(err);
    throw err;
  }
}

function toastSetupGuidance(summary: SettingsSummary | null | undefined): void {
  const guidance = summary?.setup_guidance?.trim();
  if (!guidance) return;
  useToastStore.getState().showToast(guidance, "info", 12000);
}

function stampClock(
  clock: SettingsDocClock,
  docs: SettingsDocument[],
  revision: number,
): SettingsDocClock {
  const next = { ...clock };
  for (const doc of docs) {
    next[doc] = doc === "excludes" ? EXCLUDES_CLOCK : revision;
  }
  return next;
}

function applyMcpListing(listing: LayeredList<McpServerItem>): Pick<
  SettingsStoreState,
  "mcpDefs" | "mcpRuntime"
> {
  return splitMcpListing(listing);
}

export const useSettingsStore = create<SettingsStore>((set, get) => {
  async function loadDocuments(
    docs: readonly SettingsDocument[],
    opts: { forceRevisioned: boolean; forceExcludes: boolean },
  ): Promise<void> {
    const state = get();
    const needed = [...new Set(docs)].filter((doc) => {
      if (doc === "excludes") {
        return opts.forceExcludes || !documentIsFresh(doc, state);
      }
      return opts.forceRevisioned || !documentIsFresh(doc, state);
    });
    if (needed.length === 0) return;

    const patch: Partial<SettingsStoreState> = {};
    const stamped: SettingsDocument[] = [];
    const tasks: Promise<void>[] = [];

    const run = (doc: SettingsDocument, work: () => Promise<void>) => {
      if (!needed.includes(doc)) return;
      stamped.push(doc);
      tasks.push(work());
    };

    run("summary", async () => {
      const summary = await getSettingsSummary();
      patch.summary = summary;
      patch.revision = summary.revision;
    });
    run("adapters", async () => {
      patch.adapters = await getAdapters();
    });
    run("providers", async () => {
      patch.providers = await getProviders();
    });
    run("models", async () => {
      patch.models = await getModels();
    });
    run("availableTools", async () => {
      patch.availableTools = await getAvailableTools();
    });
    run("customTools", async () => {
      patch.customTools = await getCustomTools();
    });
    run("mcp", async () => {
      Object.assign(patch, applyMcpListing(await getMcpServers()));
    });
    run("log", async () => {
      patch.log = await getLog();
    });
    run("websearch", async () => {
      patch.websearch = await getWebSearch();
    });
    run("excludes", async () => {
      patch.excludes = await getExcludes();
    });
    run("agents", async () => {
      const agentIds = await loadSettingsAgentIds();
      const agents: Record<string, AgentProfile> = {};
      await Promise.all(
        agentIds.map(async (id) => {
          agents[id] = await getAgent(id);
        }),
      );
      patch.agentIds = agentIds;
      patch.agents = agents;
      const selected = get().selectedAgentId;
      patch.selectedAgentId = agentIds.includes(selected) ? selected : (agentIds[0] ?? "default");
    });

    await Promise.all(tasks);
    const revision = patch.revision ?? get().revision;
    set({
      ...patch,
      revision,
      docClock: stampClock(get().docClock, stamped, revision),
    });
  }

  return {
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
    mcpDefs: null,
    mcpRuntime: null,
    agentIds: ["default"],
    selectedAgentId: "default",
    agents: {},
    log: null,
    excludes: null,
    loadError: null,
    persistStatus: "idle",
    docClock: {},

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
        void get().ensureSectionLoaded(get().section, true);
      }
    },

    handleWorkspaceChange: (paths) => {
      const clock = { ...get().docClock };
      let touched = false;
      if (paths.some(isWorkspaceExcludesPath)) {
        delete clock.excludes;
        touched = true;
      }
      if (paths.some(isWorkspaceMcpPath)) {
        delete clock.mcp;
        touched = true;
      }
      if (paths.some(isWorkspaceCustomToolsPath)) {
        delete clock.customTools;
        delete clock.availableTools;
        touched = true;
      }
      if (!touched) return;
      set({ docClock: clock });
      if (!get().open) return;
      const section = get().section;
      const docs = SECTION_DOCUMENTS[section];
      const reload =
        (docs.includes("excludes") && paths.some(isWorkspaceExcludesPath)) ||
        (docs.includes("mcp") && paths.some(isWorkspaceMcpPath)) ||
        ((docs.includes("customTools") || docs.includes("availableTools")) &&
          paths.some(isWorkspaceCustomToolsPath));
      if (reload) {
        void get().ensureSectionLoaded(section);
      }
    },

    notifySetupIfNeeded: async () => {
      try {
        const summary = await getSettingsSummary();
        set({
          summary,
          revision: summary.revision,
          docClock: stampClock(get().docClock, ["summary"], summary.revision),
        });
        toastSetupGuidance(summary);
      } catch {
        // Connect path already surfaces transport errors elsewhere.
      }
    },

    ensureSectionLoaded: async (section, force = false) => {
      const docs = SECTION_DOCUMENTS[section];
      if (docs.length === 0) return;
      const flight = ++loadFlight;
      const requested = section;
      try {
        await loadDocuments(docs, { forceRevisioned: force, forceExcludes: false });
        if (section === "models") {
          void loadDocuments(["agents"], {
            forceRevisioned: false,
            forceExcludes: false,
          }).catch(() => {
            /* referenced-model check is best-effort */
          });
        }
        if (get().section === requested && flight === loadFlight) {
          set({ loadError: null });
        }
      } catch (err) {
        if (get().section !== requested || flight !== loadFlight) return;
        const message = err instanceof Error ? err.message : "Failed to load settings";
        set({ loadError: message });
        useToastStore.getState().showToast(message, "error");
      }
    },

    refreshAgents: async () => {
      await loadDocuments(["agents"], { forceRevisioned: true, forceExcludes: false });
    },

    setSelectedAgentId: (id) => set({ selectedAgentId: id }),

    saveProviders: (providers) =>
      withTurnGuard(async () => {
        const { revision } = await putProviders(providers);
        const next = await getProviders();
        set({
          revision,
          providers: next,
          docClock: stampClock(get().docClock, ["providers"], revision),
        });
      }),

    saveWebSearch: (body) =>
      withTurnGuard(async () => {
        const { revision } = await putWebSearch(body);
        const websearch = await getWebSearch();
        set({
          revision,
          websearch,
          docClock: stampClock(get().docClock, ["websearch"], revision),
        });
      }),

    saveModels: (models) =>
      withTurnGuard(async () => {
        const { revision } = await putModels(models);
        set({
          revision,
          models,
          docClock: stampClock(get().docClock, ["models"], revision),
        });
        void useSessionStore.getState().refreshAvailableModels();
      }),

    saveCustomTool: (id, def, scope = "global") =>
      withTurnGuard(async () => {
        const { revision } = await putCustomTool(id, def, scope);
        const [customTools, availableTools] = await Promise.all([
          getCustomTools(),
          getAvailableTools(),
        ]);
        set({
          revision,
          customTools,
          availableTools,
          docClock: stampClock(get().docClock, ["customTools", "availableTools"], revision),
        });
      }),

    removeCustomTool: (id, scope = "global") =>
      withTurnGuard(async () => {
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
          docClock: stampClock(get().docClock, ["customTools", "availableTools"], revision),
        });
      }),

    saveMcpServer: (id, def, scope = "global") =>
      withTurnGuard(async () => {
        const { revision } = await putMcpServer(id, def, scope);
        const listing = await getMcpServers();
        const availableTools = await getAvailableTools();
        set({
          revision,
          availableTools,
          ...applyMcpListing(listing),
          docClock: stampClock(get().docClock, ["mcp", "availableTools"], revision),
        });
      }),

    removeMcpServer: (id, scope = "global") =>
      withTurnGuard(async () => {
        const { revision } = await deleteMcpServer(id, scope);
        const listing = await getMcpServers();
        const availableTools = await getAvailableTools();
        await get().refreshAgents();
        set({
          revision,
          availableTools,
          ...applyMcpListing(listing),
          docClock: stampClock(get().docClock, ["mcp", "availableTools"], revision),
        });
      }),

    startMcpServer: async (id, def, scope = "global") => {
      const result = await requestMcpStart(id, def, scope);
      const listing = await getMcpServers();
      set({
        ...applyMcpListing(listing),
        docClock: stampClock(get().docClock, ["mcp"], get().revision),
      });
      return result;
    },

    restartMcpServer: async (id, def, scope = "global") => {
      const result = await requestMcpRestart(id, def, scope);
      const listing = await getMcpServers();
      set({
        ...applyMcpListing(listing),
        docClock: stampClock(get().docClock, ["mcp"], get().revision),
      });
      return result;
    },

    stopMcpServer: async (id, scope = "global") => {
      const result = await requestMcpStop(id, scope);
      const listing = await getMcpServers();
      set({
        ...applyMcpListing(listing),
        docClock: stampClock(get().docClock, ["mcp"], get().revision),
      });
      return result;
    },

    saveAgent: (id, profile) =>
      withTurnGuard(async () => {
        const synced = withSyncedToolSeries(profile);
        const { revision } = await putAgent(id, synced);
        set((s) => ({
          revision,
          agents: { ...s.agents, [id]: synced },
          docClock: stampClock(s.docClock, ["agents"], revision),
        }));
      }),

    createAgent: async (id, profile) => {
      await get().saveAgent(id, profile);
      await get().refreshAgents();
      set({ selectedAgentId: id });
    },

    removeAgent: (id) =>
      withTurnGuard(async () => {
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
            : (agentIds[0] ?? "default"),
          docClock: stampClock(get().docClock, ["agents"], revision),
        });
      }),

    saveLog: (level) =>
      withTurnGuard(async () => {
        const { revision } = await putLog(level);
        set({
          revision,
          log: { level },
          docClock: stampClock(get().docClock, ["log"], revision),
        });
      }),

    saveExcludes: (body) =>
      withTurnGuard(async () => {
        const excludes = await putExcludes(body);
        set({
          excludes,
          docClock: stampClock(get().docClock, ["excludes"], get().revision),
        });
      }),
  };
});

attachSiblingStores({ settings: useSettingsStore });

export function mcpServersFromStore(state: {
  mcpDefs: LayeredList<McpDefItem> | null;
  mcpRuntime: LayeredMcpRuntime | null;
}): LayeredList<McpServerItem> {
  return mergeLayeredMcp(state.mcpDefs, state.mcpRuntime);
}

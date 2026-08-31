import { waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { useSettingsStore } from "./settingsStore";
import { useToastStore } from "./toastStore";
import { registerSettingsFlush } from "../lib/settingsPersist";

vi.mock("../api/settings", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../api/settings")>();
  return {
    ...actual,
    getSettingsSummary: vi.fn(),
    getAdapters: vi.fn(),
    getProviders: vi.fn(),
    getModels: vi.fn(),
    getAgent: vi.fn(),
    loadSettingsAgentIds: vi.fn(),
    getMcpServers: vi.fn(),
    getAvailableTools: vi.fn(),
    getCustomTools: vi.fn(),
    getLog: vi.fn(),
    getExcludes: vi.fn(),
    getWebSearch: vi.fn(),
  };
});

vi.mock("../api/workspace", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../api/workspace")>();
  return {
    ...actual,
    getEnginesDetail: vi.fn(),
    getEngines: vi.fn(),
  };
});

import {
  getAdapters,
  getAgent,
  getAvailableTools,
  getCustomTools,
  getExcludes,
  getMcpServers,
  getModels,
  getProviders,
  getSettingsSummary,
  loadSettingsAgentIds,
} from "../api/settings";
import { getEnginesDetail } from "../api/workspace";

const mockedSummary = vi.mocked(getSettingsSummary);
const mockedAdapters = vi.mocked(getAdapters);
const mockedProviders = vi.mocked(getProviders);
const mockedModels = vi.mocked(getModels);
const mockedAgent = vi.mocked(getAgent);
const mockedAgentIds = vi.mocked(loadSettingsAgentIds);
const mockedMcp = vi.mocked(getMcpServers);
const mockedTools = vi.mocked(getAvailableTools);
const mockedCustom = vi.mocked(getCustomTools);
const mockedExcludes = vi.mocked(getExcludes);
const mockedDetail = vi.mocked(getEnginesDetail);

function summary(revision: number) {
  return {
    revision,
    provider_endpoint: null,
    model_count: 0,
    agent_count: 1,
    catalog_count: 0,
    log_level: "info",
    effective_next_turn: true,
    restart_required: false,
  };
}

const emptyExcludes = {
  files_exclude: [] as string[],
  search_exclude: [] as string[],
  watcher_exclude: [] as string[],
  git_ignore: true,
  explorer_git_ignore: false,
  defaults: {
    files_exclude: [] as string[],
    search_exclude: [] as string[],
    watcher_exclude: [] as string[],
    git_ignore: true,
    explorer_git_ignore: false,
  },
};

beforeEach(() => {
  useSettingsStore.setState({
    open: false,
    section: "connection",
    revision: 0,
    summary: null,
    adapters: [],
    providers: null,
    models: null,
    availableTools: null,
    customTools: null,
    mcpDefs: null,
    mcpRuntime: null,
    loadError: null,
    persistStatus: "idle",
    docClock: {},
    excludes: null,
  });
  useToastStore.setState({ toasts: [] });
  mockedSummary.mockReset().mockResolvedValue(summary(1));
  mockedAdapters.mockReset().mockResolvedValue([]);
  mockedProviders.mockReset().mockResolvedValue({});
  mockedModels.mockReset().mockResolvedValue({});
  mockedAgent.mockReset();
  mockedAgentIds.mockReset().mockResolvedValue(["default"]);
  mockedMcp.mockReset().mockResolvedValue({ global: [], workspace: [] });
  mockedTools.mockReset().mockResolvedValue([]);
  mockedCustom.mockReset().mockResolvedValue({ global: [], workspace: [] });
  mockedExcludes.mockReset().mockResolvedValue(emptyExcludes);
  mockedDetail.mockReset();
});

afterEach(() => {
  vi.useRealTimers();
});

describe("ensureSectionLoaded", () => {
  it("opens Provider without fetching agents, MCP, or engines/detail", async () => {
    useSettingsStore.getState().openSettings("connection");
    await waitFor(() => {
      expect(useSettingsStore.getState().providers).toEqual({});
    });

    expect(useSettingsStore.getState().open).toBe(true);
    expect(mockedProviders).toHaveBeenCalled();
    expect(mockedAgent).not.toHaveBeenCalled();
    expect(mockedMcp).not.toHaveBeenCalled();
    expect(mockedCustom).not.toHaveBeenCalled();
    expect(mockedDetail).not.toHaveBeenCalled();
    expect(useSettingsStore.getState().providers).toEqual({});
  });

  it("does not fetch agents when opening Models — only after the models docs land", async () => {
    mockedAgent.mockResolvedValue({
      role: "primary",
      model_ref: "",
      system_prompt: "",
      temperature: 0,
      max_steps: 1,
      description: "",
      tools: {},
      allowed_subagents: [],
    });
    useSettingsStore.getState().openSettings("models");
    await waitFor(() => {
      expect(useSettingsStore.getState().models).toEqual({});
    });
    expect(mockedModels).toHaveBeenCalled();
    await waitFor(() => {
      expect(mockedAgentIds).toHaveBeenCalled();
    });
  });

  it("refreshes the current section after a remote change even while persistStatus is saving", async () => {
    useSettingsStore.setState({
      open: true,
      section: "connection",
      persistStatus: "saving",
      providers: {},
      docClock: { providers: 1, adapters: 1, summary: 1 },
      revision: 1,
    });
    mockedProviders.mockResolvedValue({
      p1: {
        id: "p1",
        adapter_id: "openai",
        label: "Remote",
        endpoint: "https://api.openai.com/v1",
        api_key: "sk-…",
        auth: "bearer",
      },
    });
    useSettingsStore.getState().onRemoteSettingsChanged({
      revision: 2,
      summary: summary(2),
    });
    await waitFor(() => {
      expect(useSettingsStore.getState().providers?.p1?.label).toBe("Remote");
    });

    expect(mockedProviders).toHaveBeenCalled();
  });

  it("does not paint a failed Provider load onto Files after switching tabs", async () => {
    mockedProviders.mockRejectedValue(new Error("provider boom"));
    useSettingsStore.getState().openSettings("connection");
    await useSettingsStore.getState().setSection("files");
    await waitFor(() => {
      expect(useSettingsStore.getState().excludes).not.toBeNull();
    });
    await Promise.resolve();
    expect(useSettingsStore.getState().section).toBe("files");
    expect(useSettingsStore.getState().loadError).toBeNull();
  });
});

describe("workspace excludes clock", () => {
  it("reloads Files when .litecode/excludes.json changes", async () => {
    useSettingsStore.setState({
      open: true,
      section: "files",
      excludes: emptyExcludes,
      docClock: { excludes: 1 },
    });
    mockedExcludes.mockResolvedValue({
      ...emptyExcludes,
      git_ignore: false,
    });
    useSettingsStore.getState().handleWorkspaceChange([".litecode/excludes.json"], "modified");
    await waitFor(() => {
      expect(useSettingsStore.getState().excludes?.git_ignore).toBe(false);
    });
  });
});

describe("settings persist toasts", () => {
  it("does not success-toast settings/changed while the dialog is open", () => {
    useToastStore.setState({ toasts: [] });
    useSettingsStore.setState({ open: true, persistStatus: "saving" });
    useSettingsStore.getState().onRemoteSettingsChanged({
      revision: 99,
      summary: summary(99),
    });
    expect(useToastStore.getState().toasts.map((t) => t.message)).not.toContain(
      "Settings changed — effective next turn",
    );
  });

  it("flushes registered persist before closing settings", async () => {
    const flush = vi.fn(async () => undefined);
    const unreg = registerSettingsFlush(flush);
    await useSettingsStore.getState().closeSettings();
    expect(flush).toHaveBeenCalledTimes(1);
    unreg();
  });
});

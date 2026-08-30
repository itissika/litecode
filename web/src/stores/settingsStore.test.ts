import { waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { useSettingsStore } from "./settingsStore";
import { useToastStore } from "./toastStore";
import { registerSettingsFlush } from "../dockview/panels/settings/persist";

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
    mcpServers: null,
    loadError: null,
    persistStatus: "idle",
    loadedRevisionBySection: {},
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

  it("refreshes the current section after a save when persistStatus is saved", async () => {
    useSettingsStore.setState({
      open: true,
      section: "connection",
      persistStatus: "saved",
      providers: {},
      loadedRevisionBySection: { connection: 1 },
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
    expect(useSettingsStore.getState().providers?.p1?.label).toBe("Remote");
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

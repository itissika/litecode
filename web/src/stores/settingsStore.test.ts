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
    getLlmSettings: vi.fn(),
    putProviderKey: vi.fn(),
    deleteProviderKey: vi.fn(),
    getAgent: vi.fn(),
    loadSettingsAgentIds: vi.fn(),
    getMcpServers: vi.fn(),
    getAvailableTools: vi.fn(),
    getCustomTools: vi.fn(),
    getLog: vi.fn(),
    getExcludes: vi.fn(),
    getEnginesDoc: vi.fn(),
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
  SettingsApiError,
  deleteProviderKey,
  getAgent,
  getAvailableTools,
  getCustomTools,
  getEnginesDoc,
  getExcludes,
  getLlmSettings,
  getMcpServers,
  getSettingsSummary,
  loadSettingsAgentIds,
  putProviderKey,
} from "../api/settings";
import type { CatalogProviderDto, LlmSettings } from "../api/settings";
import { getEnginesDetail } from "../api/workspace";

const mockedSummary = vi.mocked(getSettingsSummary);
const mockedLlm = vi.mocked(getLlmSettings);
const mockedPutKey = vi.mocked(putProviderKey);
const mockedDeleteKey = vi.mocked(deleteProviderKey);
const mockedAgent = vi.mocked(getAgent);
const mockedAgentIds = vi.mocked(loadSettingsAgentIds);
const mockedMcp = vi.mocked(getMcpServers);
const mockedTools = vi.mocked(getAvailableTools);
const mockedCustom = vi.mocked(getCustomTools);
const mockedExcludes = vi.mocked(getExcludes);
const mockedEngines = vi.mocked(getEnginesDoc);
const mockedDetail = vi.mocked(getEnginesDetail);

function summary(revision: number) {
  return {
    revision,
    configured_provider_count: 0,
    active_model_count: 0,
    agent_count: 1,
    log_level: "info",
    effective_next_turn: true,
    restart_required: false,
  };
}

function provider(patch: Partial<CatalogProviderDto> = {}): CatalogProviderDto {
  return {
    id: "openai",
    name: "OpenAI",
    visible: true,
    configured: false,
    masked_api_key: null,
    endpoint: "https://api.openai.com/v1",
    endpoint_type: "responses",
    models: [],
    ...patch,
  };
}

const llmDoc: LlmSettings = {
  catalog_path: "C:\\x\\provider-catalog.toml",
  revision: 1,
  providers: [provider()],
  active_models: [],
};

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
    llm: null,
    availableTools: null,
    customTools: null,
    mcpDefs: null,
    mcpRuntime: null,
    loadError: null,
    persistByDoc: {},
    docClock: {},
    excludes: null,
    engines: null,
  });
  useToastStore.setState({ toasts: [] });
  mockedSummary.mockReset().mockResolvedValue(summary(1));
  mockedLlm.mockReset().mockResolvedValue(llmDoc);
  mockedPutKey.mockReset().mockResolvedValue({ revision: 2, docs: ["llm"] });
  mockedDeleteKey.mockReset().mockResolvedValue({ revision: 2, docs: ["llm"] });
  mockedAgent.mockReset();
  mockedAgentIds.mockReset().mockResolvedValue(["default"]);
  mockedMcp.mockReset().mockResolvedValue({ global: [], workspace: [] });
  mockedTools.mockReset().mockResolvedValue([]);
  mockedCustom.mockReset().mockResolvedValue({ global: [], workspace: [] });
  mockedExcludes.mockReset().mockResolvedValue(emptyExcludes);
  mockedEngines.mockReset().mockResolvedValue({
    version: 1,
    lsp: { desired: false, servers: [] },
    retrieval: { desired: false },
  });
  mockedDetail.mockReset();
});

afterEach(() => {
  vi.useRealTimers();
});

describe("ensureSectionLoaded", () => {
  it("opens Provider with summary + llm only", async () => {
    useSettingsStore.getState().openSettings("connection");
    await waitFor(() => {
      expect(useSettingsStore.getState().llm).not.toBeNull();
    });

    expect(useSettingsStore.getState().open).toBe(true);
    expect(mockedLlm).toHaveBeenCalled();
    expect(mockedAgent).not.toHaveBeenCalled();
    expect(mockedMcp).not.toHaveBeenCalled();
    expect(mockedCustom).not.toHaveBeenCalled();
    expect(mockedDetail).not.toHaveBeenCalled();
    expect(useSettingsStore.getState().llm?.catalog_path).toContain(
      "provider-catalog.toml",
    );
  });

  it("loads llm + agents when opening Agents", async () => {
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
    useSettingsStore.getState().openSettings("agents");
    await waitFor(() => {
      expect(mockedAgentIds).toHaveBeenCalled();
    });
    expect(mockedLlm).toHaveBeenCalled();
  });

  it("refreshes the llm document after a remote change while the dialog is open", async () => {
    useSettingsStore.setState({
      open: true,
      section: "connection",
      persistByDoc: { llm: "saving" },
      llm: llmDoc,
      docClock: { llm: 1, summary: 1 },
      revision: 1,
    });
    mockedLlm.mockResolvedValue({
      ...llmDoc,
      revision: 2,
      providers: [provider({ configured: true, masked_api_key: "sk-***remote" })],
    });
    useSettingsStore.getState().onRemoteSettingsChanged({
      revision: 2,
      docs: ["llm"],
      summary: summary(2),
    });
    await waitFor(() => {
      expect(useSettingsStore.getState().llm?.providers[0]?.masked_api_key).toBe(
        "sk-***remote",
      );
    });

    expect(mockedLlm).toHaveBeenCalled();
  });

  it("does not paint a failed Provider load onto Files after switching tabs", async () => {
    mockedLlm.mockRejectedValue(new Error("catalog boom"));
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

describe("reopen settings rereads gate docs", () => {
  it("refetches MCP after close even when generation did not move", async () => {
    useSettingsStore.setState({
      open: false,
      revision: 1,
      mcpDefs: { global: [], workspace: [] },
      mcpRuntime: { global: {}, workspace: {} },
      docClock: { mcp: 1 },
    });
    mockedMcp.mockResolvedValue({
      global: [],
      workspace: [{ id: "ws", command: "uvx", origin: "workspace" }],
    });
    useSettingsStore.getState().openSettings("mcp");
    await waitFor(() => {
      expect(useSettingsStore.getState().mcpDefs?.workspace).toEqual([
        expect.objectContaining({ id: "ws", command: "uvx" }),
      ]);
    });
    expect(mockedMcp).toHaveBeenCalled();
  });

  it("refetches excludes after close even when the excludes clock is set", async () => {
    useSettingsStore.setState({
      open: false,
      revision: 1,
      excludes: emptyExcludes,
      docClock: { excludes: 1 },
    });
    mockedExcludes.mockResolvedValue({
      ...emptyExcludes,
      git_ignore: false,
    });
    useSettingsStore.getState().openSettings("files");
    await waitFor(() => {
      expect(useSettingsStore.getState().excludes?.git_ignore).toBe(false);
    });
    expect(mockedExcludes).toHaveBeenCalled();
  });

  it("refetches custom tools when switching to that tab after a closed reopen", async () => {
    useSettingsStore.setState({
      open: false,
      revision: 1,
      llm: llmDoc,
      customTools: { global: [], workspace: [] },
      docClock: { llm: 1, customTools: 1, summary: 1 },
    });
    mockedCustom.mockResolvedValue({
      global: [],
      workspace: [
        {
          name: "ws_tool",
          description: "from disk",
          schema: { type: "object", properties: {} },
          command: "echo",
        },
      ],
    });
    useSettingsStore.getState().openSettings("connection");
    await waitFor(() => {
      expect(useSettingsStore.getState().open).toBe(true);
    });
    await useSettingsStore.getState().setSection("custom-tools");
    await waitFor(() => {
      expect(useSettingsStore.getState().customTools?.workspace).toEqual([
        expect.objectContaining({ name: "ws_tool" }),
      ]);
    });
    expect(mockedCustom).toHaveBeenCalled();
  });

  it("does not drop clocks when settings is already open", async () => {
    useSettingsStore.setState({
      open: true,
      section: "files",
      revision: 1,
      excludes: emptyExcludes,
      docClock: { excludes: 1 },
    });
    mockedExcludes.mockClear();
    useSettingsStore.getState().openSettings("files");
    await Promise.resolve();
    expect(mockedExcludes).not.toHaveBeenCalled();
  });
});

describe("workspace excludes clock", () => {
  it("does not hydrate Files from watcher events", async () => {
    useSettingsStore.setState({
      open: true,
      section: "files",
      excludes: emptyExcludes,
      docClock: { excludes: 1 },
    });
    mockedExcludes.mockClear();
    useSettingsStore.getState().handleWorkspaceChange([".litecode/excludes.json"], "modified");
    await Promise.resolve();
    expect(mockedExcludes).not.toHaveBeenCalled();
    expect(useSettingsStore.getState().excludes?.git_ignore).toBe(true);
  });

  it("does not hydrate MCP from watcher events", async () => {
    useSettingsStore.setState({
      open: true,
      section: "mcp",
      mcpDefs: { global: [], workspace: [] },
      mcpRuntime: { global: {}, workspace: {} },
      docClock: { mcp: 1 },
    });
    mockedMcp.mockClear();
    useSettingsStore.getState().handleWorkspaceChange([".litecode/mcp.json"], "modified");
    await Promise.resolve();
    expect(mockedMcp).not.toHaveBeenCalled();
    expect(useSettingsStore.getState().mcpDefs?.workspace).toEqual([]);
  });

  it("does not hydrate custom tools from watcher events", async () => {
    useSettingsStore.setState({
      open: true,
      section: "custom-tools",
      customTools: { global: [], workspace: [] },
      docClock: { customTools: 1 },
    });
    mockedCustom.mockClear();
    useSettingsStore
      .getState()
      .handleWorkspaceChange([".litecode/custom_tools.json"], "modified");
    await Promise.resolve();
    expect(mockedCustom).not.toHaveBeenCalled();
    expect(useSettingsStore.getState().customTools?.workspace).toEqual([]);
  });
});

describe("settings persist toasts", () => {
  it("does not success-toast settings/changed while the dialog is open", () => {
    useToastStore.setState({ toasts: [] });
    useSettingsStore.setState({ open: true, persistByDoc: { llm: "saving" } });
    useSettingsStore.getState().onRemoteSettingsChanged({
      revision: 99,
      docs: [],
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

describe("provider credential actions", () => {
  it("saves a key through the key API and reloads the llm document", async () => {
    mockedPutKey.mockResolvedValue({ revision: 4, docs: ["llm"] });
    mockedLlm.mockResolvedValue({
      ...llmDoc,
      revision: 4,
      providers: [provider({ configured: true, masked_api_key: "sk-***test" })],
    });

    await useSettingsStore.getState().saveProviderKey("openai", "sk-test");

    expect(mockedPutKey).toHaveBeenCalledTimes(1);
    expect(mockedPutKey).toHaveBeenCalledWith("openai", "sk-test");
    const state = useSettingsStore.getState();
    expect(state.revision).toBe(4);
    expect(state.llm?.providers[0]?.configured).toBe(true);
    expect(state.docClock.llm).toBe(4);
  });

  it("never issues a PUT for an empty key", async () => {
    await expect(
      useSettingsStore.getState().saveProviderKey("openai", "   "),
    ).rejects.toBeInstanceOf(SettingsApiError);
    expect(mockedPutKey).not.toHaveBeenCalled();
  });

  it("deletes a credential and returns the provider to the add list", async () => {
    useSettingsStore.setState({
      llm: {
        ...llmDoc,
        providers: [provider({ configured: true, masked_api_key: "sk-***x" })],
      },
    });
    mockedDeleteKey.mockResolvedValue({ revision: 5, docs: ["llm"] });
    mockedLlm.mockResolvedValue({ ...llmDoc, revision: 5, providers: [provider()] });

    await useSettingsStore.getState().removeProviderKey("openai");

    expect(mockedDeleteKey).toHaveBeenCalledWith("openai");
    const state = useSettingsStore.getState();
    expect(state.revision).toBe(5);
    expect(state.llm?.providers[0]?.configured).toBe(false);
    expect(state.llm?.providers[0]?.masked_api_key).toBeNull();
  });
});


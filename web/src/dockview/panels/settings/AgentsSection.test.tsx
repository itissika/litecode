import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type {
  AgentProfile,
  AgentToolBinding,
  AvailableTool,
  CatalogModelDto,
  LlmSettings,
} from "../../../api/settings";
import { useSettingsStore } from "../../../stores/settingsStore";
import { AgentsSection } from "./AgentsSection";

const model: CatalogModelDto = {
  ref: "prov/m1",
  id: "gpt-4o",
  label: "GPT",
  provider_id: "prov",
  provider_name: "Prov",
  context_window: 200_000,
  context_window_max: 400_000,
  modalities: ["text"],
  tool_call: true,
  json_output: false,
  enabled: true,
};

const llmDoc: LlmSettings = {
  catalog_path: "C:\\x\\provider-catalog.toml",
  revision: 1,
  providers: [
    {
      id: "prov",
      name: "Prov",
      visible: true,
      configured: true,
      masked_api_key: "sk-***x",
      endpoint: "https://prov.example/v1",
      endpoint_type: "responses",
      models: [model],
    },
  ],
  active_models: [model],
};

function profile(patch: Partial<AgentProfile> = {}): AgentProfile {
  return {
    role: "primary",
    model_ref: "prov/m1",
    system_prompt: "",
    temperature: 0.7,
    max_steps: 50,
    description: "",
    tools: {},
    allowed_subagents: [],
    ...patch,
  };
}

describe("AgentsSection persist UX", () => {
  const saveAgent = vi.fn(async () => undefined);
  const createAgent = vi.fn(async () => undefined);
  const removeAgent = vi.fn(async () => undefined);
  const refreshAgents = vi.fn(async () => undefined);

  beforeEach(() => {
    saveAgent.mockClear();
    createAgent.mockClear();
    removeAgent.mockClear();
    refreshAgents.mockClear();
    useSettingsStore.setState({
      llm: llmDoc,
      availableTools: [],
      mcpDefs: { global: [], workspace: [] },
      mcpRuntime: { global: {}, workspace: {} },
      agentIds: ["default"],
      selectedAgentId: "default",
      agents: { default: profile() },
      persistByDoc: {},
      saveAgent,
      createAgent,
      removeAgent,
      refreshAgents,
    });
  });

  afterEach(() => {
    cleanup();
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it("does not auto-save or show Fix fields to save when adding an agent", async () => {
    vi.useFakeTimers();
    render(<AgentsSection />);
    fireEvent.click(screen.getByRole("button", { name: "Add agent" }));
    await vi.advanceTimersByTimeAsync(400);
    expect(screen.getByPlaceholderText("my_agent")).toBeTruthy();
    expect(screen.queryByText("Fix fields to save")).toBeNull();
    expect(saveAgent).not.toHaveBeenCalled();
    expect(createAgent).not.toHaveBeenCalled();
  });

  it("creates once the new agent has an id", async () => {
    render(<AgentsSection />);
    fireEvent.click(screen.getByRole("button", { name: "Add agent" }));
    fireEvent.change(screen.getByPlaceholderText("my_agent"), {
      target: { value: "my_agent" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Create" }));
    await waitFor(() => {
      expect(createAgent).toHaveBeenCalledWith("my_agent", expect.any(Object));
    });
    expect(saveAgent).not.toHaveBeenCalled();
  });

  it("PUTs the selected agent after an edit", async () => {
    vi.useFakeTimers();
    render(<AgentsSection />);
    const description = screen.getAllByRole("textbox").find(
      (el) => (el as HTMLInputElement).type !== "number" && (el as HTMLTextAreaElement).rows == null,
    );
    expect(description).toBeTruthy();
    fireEvent.change(description!, { target: { value: "Helper" } });
    await vi.advanceTimersByTimeAsync(400);
    expect(saveAgent).toHaveBeenCalledWith(
      "default",
      expect.objectContaining({ description: "Helper" }),
    );
  });

  it("hides unused fields for the hidden compaction agent", () => {
    useSettingsStore.setState({
      agentIds: ["default", "compaction"],
      selectedAgentId: "compaction",
      agents: {
        default: profile(),
        compaction: profile({
          role: "hidden",
          system_prompt: "builtin:compaction",
          description: "",
        }),
      },
    });
    render(<AgentsSection />);
    expect(screen.getByText("Model")).toBeTruthy();
    expect(screen.queryByText("Type")).toBeNull();
    expect(screen.queryByText("Description")).toBeNull();
    expect(screen.queryByText("System prompt")).toBeNull();
    expect(screen.queryByText("Max steps")).toBeNull();
    expect(
      screen.getByText("Compaction only assigns a model. Prompt, tools, and max steps are built in."),
    ).toBeTruthy();
  });

  it("deletes a non-protected agent after confirm", async () => {
    vi.stubGlobal("confirm", vi.fn(() => true));
    useSettingsStore.setState({
      agentIds: ["helper"],
      selectedAgentId: "helper",
      agents: { helper: profile({ role: "subagent" }) },
    });
    render(<AgentsSection />);
    fireEvent.click(screen.getByRole("button", { name: "Delete" }));
    await waitFor(() => {
      expect(removeAgent).toHaveBeenCalledWith("helper");
    });
  });
});

const lspTool: AvailableTool = { id: "lsp", kind: "engine", origin: "workspace" };
const readTool: AvailableTool = { id: "read", kind: "core", origin: "builtin" };
const mcpDemoTool: AvailableTool = { id: "mcp_demo", kind: "mcp", origin: "workspace" };

describe("AgentsSection subagent tool cards", () => {
  const saveAgent = vi.fn(async (_id: string, _next: AgentProfile) => undefined);
  const createAgent = vi.fn(async () => undefined);
  const removeAgent = vi.fn(async () => undefined);
  const refreshAgents = vi.fn(async () => undefined);

  beforeEach(() => {
    saveAgent.mockClear();
    useSettingsStore.setState({
      llm: llmDoc,
      availableTools: [readTool, lspTool, mcpDemoTool],
      mcpDefs: { global: [], workspace: [] },
      mcpRuntime: { global: {}, workspace: {} },
      agentIds: ["helper"],
      selectedAgentId: "helper",
      agents: { helper: profile({ role: "subagent" }) },
      persistByDoc: {},
      saveAgent,
      createAgent,
      removeAgent,
      refreshAgents,
    });
  });

  afterEach(() => {
    cleanup();
    vi.useRealTimers();
  });

  it("renders per-tool preset cards with a deny-semantics note, not checkboxes", () => {
    render(<AgentsSection />);
    // Full card rows (clickable), one per bindable tool — not a checkbox list.
    expect(screen.getByRole("button", { name: /read tool binding, disabled/i })).toBeTruthy();
    expect(screen.getByRole("button", { name: /lsp tool binding, disabled/i })).toBeTruthy();
    expect(screen.queryByRole("checkbox")).toBeNull();
    // Configurable tools expose the ALL/SAFE preset control.
    const readPreset = screen.getByRole("group", { name: "read preset" });
    expect(within(readPreset).getByRole("button", { name: "SAFE" })).toBeTruthy();
    expect(within(readPreset).getByRole("button", { name: "ALL" })).toBeTruthy();
    // MCP server bindings expose the per-server tool visibility picker.
    expect(
      screen.getByRole("button", { name: "Select visible tools for MCP server demo" }),
    ).toBeTruthy();
    // Guidance on Ask -> deny semantics for subagent turns.
    expect(screen.getByText(/can't ask for approval/i)).toBeTruthy();
  });

  it("persists a SAFE preset picked on a subagent tool card", async () => {
    vi.useFakeTimers();
    render(<AgentsSection />);
    fireEvent.click(screen.getByRole("button", { name: /read tool binding, disabled/i }));
    const readPreset = screen.getByRole("group", { name: "read preset" });
    fireEvent.click(within(readPreset).getByRole("button", { name: "SAFE" }));
    await vi.advanceTimersByTimeAsync(400);
    expect(saveAgent).toHaveBeenCalledTimes(1);
    const payload = saveAgent.mock.calls[0][1] as AgentProfile;
    expect(payload.tools.read).toEqual(
      expect.objectContaining({ enabled: true, last_applied_preset: "SAFE" }),
    );
  });
});

describe("AgentsSection LSP bind persist loop", () => {
  const saveAgent = vi.fn(async (_id: string, _next: AgentProfile) => undefined);

  beforeEach(() => {
    saveAgent.mockReset();
    useSettingsStore.setState({
      llm: llmDoc,
      availableTools: [readTool, lspTool],
      mcpDefs: { global: [], workspace: [] },
      mcpRuntime: { global: {}, workspace: {} },
      agentIds: ["default"],
      selectedAgentId: "default",
      agents: { default: profile() },
      persistByDoc: {},
      revision: 1,
      saveAgent,
      createAgent: vi.fn(async () => undefined),
      removeAgent: vi.fn(async () => undefined),
      refreshAgents: vi.fn(async () => undefined),
    });
  });

  afterEach(() => {
    cleanup();
    vi.useRealTimers();
  });

  it("does not keep PUT-ing after a settings_changed reload of the same bind", async () => {
    vi.useFakeTimers();
    saveAgent.mockImplementation(async (id: string, next: AgentProfile) => {
      useSettingsStore.setState((s) => ({
        revision: s.revision + 1,
        agents: { ...s.agents, [id]: next },
      }));
    });
    render(<AgentsSection />);
    fireEvent.click(
      screen.getByRole("button", {
        name: /lsp tool binding, disabled/i,
      }),
    );
    await vi.advanceTimersByTimeAsync(400);
    expect(saveAgent).toHaveBeenCalledTimes(1);

    // Echo of PUT: WS settings_changed reloads the agent (new object, extra
    // policy defaults the server expands). Must converge, not rev++ forever.
    const saved = saveAgent.mock.calls[0][1] as AgentProfile;
    useSettingsStore.setState({
      persistByDoc: useSettingsStore.getState().persistByDoc,
      agents: {
        default: {
          ...saved,
          tools: {
            ...saved.tools,
            lsp: {
              ...saved.tools.lsp,
              enabled: true,
              last_applied_preset: "ALL",
              policy: { default: "allow", default_id: "default", rules: [] },
            },
          },
        },
      },
      availableTools: [readTool, lspTool],
      revision: useSettingsStore.getState().revision,
    });
    await vi.advanceTimersByTimeAsync(400);
    await vi.advanceTimersByTimeAsync(800);
    expect(saveAgent.mock.calls.length).toBeLessThan(3);
  });

  it("does not storm saves when lsp availability flickers after a bind toggle", async () => {
    vi.useFakeTimers();
    let lspListed = true;
    let lastLsp: AgentToolBinding = { enabled: true, last_applied_preset: "ALL" };
    saveAgent.mockImplementation(async (id: string, next: AgentProfile) => {
      if (next.tools.lsp) lastLsp = next.tools.lsp;
      const merged: AgentProfile = {
        ...next,
        tools: { ...next.tools, lsp: lastLsp },
      };
      queueMicrotask(() => {
        lspListed = !lspListed;
        useSettingsStore.setState((s) => ({
          revision: s.revision + 1,
          agents: { ...s.agents, [id]: merged },
          availableTools: lspListed ? [readTool, lspTool] : [readTool],
        }));
      });
    });
    render(<AgentsSection />);
    fireEvent.click(
      screen.getByRole("button", {
        name: /lsp tool binding, disabled/i,
      }),
    );
    for (let i = 0; i < 8; i++) {
      await vi.advanceTimersByTimeAsync(400);
      await Promise.resolve();
    }
    expect(saveAgent.mock.calls.length).toBeLessThan(4);
  });

  it("hides bindings that are not in this workspace catalog", () => {
    useSettingsStore.setState({
      availableTools: [readTool, lspTool],
      agents: {
        default: profile({
          tools: {
            lsp: { enabled: true, last_applied_preset: "ALL" },
            mcp_other_ws: { enabled: true, last_applied_preset: null },
          },
        }),
      },
    });
    render(<AgentsSection />);
    expect(screen.getByText("lsp")).toBeTruthy();
    expect(screen.queryByText("mcp_other_ws")).toBeNull();
    expect(screen.queryByText("unavailable")).toBeNull();
  });

  it("keeps other-workspace bindings on PUT and does not loop when GET reshuffles keys", async () => {
    vi.useFakeTimers();
    useSettingsStore.setState({
      availableTools: [readTool, lspTool],
      agents: {
        default: profile({
          tools: {
            mcp_other_ws: { enabled: true, last_applied_preset: null },
          },
        }),
      },
    });
    saveAgent.mockImplementation(async (id: string, next: AgentProfile) => {
      const keys = Object.keys(next.tools).reverse();
      const tools: Record<string, AgentToolBinding> = {};
      for (const key of keys) {
        const binding = next.tools[key];
        tools[key] = {
          ...binding,
          policy: binding.policy ?? { default: "allow", default_id: "default", rules: [] },
          path_mode: binding.path_mode ?? "unrestricted",
          last_applied_preset: binding.last_applied_preset ?? null,
          allowed_tools: binding.allowed_tools ?? null,
        };
      }
      useSettingsStore.setState((s) => ({
        revision: s.revision + 1,
        agents: { ...s.agents, [id]: { ...next, tools } },
      }));
    });
    render(<AgentsSection />);
    fireEvent.click(
      screen.getByRole("button", {
        name: /lsp tool binding, disabled/i,
      }),
    );
    await vi.advanceTimersByTimeAsync(400);
    expect(saveAgent).toHaveBeenCalledTimes(1);
    expect(saveAgent.mock.calls[0][1].tools.mcp_other_ws).toEqual({
      enabled: true,
      last_applied_preset: null,
    });
    for (let i = 0; i < 6; i++) {
      await vi.advanceTimersByTimeAsync(400);
      await Promise.resolve();
    }
    expect(saveAgent.mock.calls.length).toBeLessThan(3);
  });
});

describe("AgentsSection model picker", () => {
  const saveAgent = vi.fn(async () => undefined);
  const createAgent = vi.fn(async () => undefined);

  const secondModel: CatalogModelDto = {
    ref: "other/m1",
    id: "m1",
    label: "Same Wire Id",
    provider_id: "other",
    provider_name: "Other",
    context_window: 100_000,
    context_window_max: 100_000,
    modalities: ["text", "image"],
    tool_call: false,
    json_output: true,
    enabled: false,
  };

  beforeEach(() => {
    saveAgent.mockClear();
    createAgent.mockClear();
    useSettingsStore.setState({
      llm: {
        ...llmDoc,
        providers: [
          ...llmDoc.providers,
          {
            id: "other",
            name: "Other",
            visible: true,
            configured: true,
            masked_api_key: "sk-***y",
            endpoint: null,
            endpoint_type: "chat_completions",
            models: [secondModel],
          },
        ],
        active_models: [model, secondModel],
      },
      availableTools: [],
      mcpDefs: { global: [], workspace: [] },
      mcpRuntime: { global: {}, workspace: {} },
      agentIds: ["default"],
      selectedAgentId: "default",
      agents: { default: profile() },
      persistByDoc: {},
      revision: 1,
      saveAgent,
      createAgent,
      removeAgent: vi.fn(async () => undefined),
      refreshAgents: vi.fn(async () => undefined),
    });
  });

  afterEach(() => {
    cleanup();
    vi.useRealTimers();
  });

  function openModelDropdown() {
    // The Select trigger shows the current option's label ("GPT").
    fireEvent.click(screen.getByRole("button", { name: /^GPT$/ }));
    const panel = document.querySelector("[data-dropdown-panel]");
    expect(panel).toBeTruthy();
    return panel as HTMLElement;
  }

  it("groups active models by provider and keeps a duplicate wire id independently selectable", async () => {
    vi.useFakeTimers();
    render(<AgentsSection />);
    const panel = openModelDropdown();

    // Provider group headers, then one option per composite ref — both
    // providers expose the wire id `m1`, yet each keeps its own ref.
    expect(within(panel).getByText("Prov")).toBeTruthy();
    expect(within(panel).getByText("Other")).toBeTruthy();
    fireEvent.click(within(panel).getByRole("button", { name: /Same Wire Id/ }));
    await vi.advanceTimersByTimeAsync(400);
    expect(saveAgent).toHaveBeenCalledWith(
      "default",
      expect.objectContaining({ model_ref: "other/m1" }),
    );
  });

  it("keeps an unknown model_ref as a disabled Missing option instead of re-pointing the agent", async () => {
    vi.useFakeTimers();
    useSettingsStore.setState({
      agents: { default: profile({ model_ref: "ghost/gone-model" }) },
    });
    render(<AgentsSection />);

    const trigger = screen.getByRole("button", { name: /Missing: ghost\/gone-model/ });
    fireEvent.click(trigger);
    const panel = document.querySelector("[data-dropdown-panel]") as HTMLElement;
    const missing = within(panel).getAllByRole("button", {
      name: /Missing: ghost\/gone-model/,
    })[0];
    expect(missing.hasAttribute("disabled")).toBe(true);

    fireEvent.click(missing);
    await vi.advanceTimersByTimeAsync(400);
    // The ref is still the missing one — nothing was silently selected.
    expect(screen.getByRole("button", { name: /Missing: ghost\/gone-model/ })).toBeTruthy();
    expect(saveAgent).not.toHaveBeenCalled();
  });

  it("defaults a new agent to the first active model", async () => {
    render(<AgentsSection />);
    fireEvent.click(screen.getByRole("button", { name: "Add agent" }));
    fireEvent.change(screen.getByPlaceholderText("my_agent"), {
      target: { value: "helper" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Create" }));
    await waitFor(() => {
      expect(createAgent).toHaveBeenCalledWith(
        "helper",
        expect.objectContaining({ model_ref: "prov/m1" }),
      );
    });
  });

  it("tells the user to configure a provider API key when no model is active", () => {
    useSettingsStore.setState({
      llm: { ...llmDoc, active_models: [] },
    });
    render(<AgentsSection />);
    expect(screen.getByText("Configure a provider API key")).toBeTruthy();
  });
});


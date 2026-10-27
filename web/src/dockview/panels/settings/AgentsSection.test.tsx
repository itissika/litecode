import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
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
    const description = screen
      .getAllByRole("textbox")
      .find(
        (el) =>
          (el as HTMLInputElement).type !== "number" &&
          (el as HTMLTextAreaElement).rows == null,
      );
    expect(description).toBeTruthy();
    fireEvent.change(description!, { target: { value: "Helper" } });
    await vi.advanceTimersByTimeAsync(400);
    expect(saveAgent).toHaveBeenCalledWith(
      "default",
      expect.objectContaining({ description: "Helper" }),
    );
  });

  it("uses a multiline description for subagents", () => {
    useSettingsStore.setState({
      agentIds: ["helper"],
      selectedAgentId: "helper",
      agents: {
        helper: profile({
          role: "subagent",
          description: "Reads the repo and reports back.",
        }),
      },
    });
    render(<AgentsSection />);
    const description = screen.getByDisplayValue(
      "Reads the repo and reports back.",
    );
    expect(description.tagName).toBe("TEXTAREA");
    expect((description as HTMLTextAreaElement).rows).toBe(4);
  });

  it("deletes a non-protected agent after confirm", async () => {
    vi.stubGlobal(
      "confirm",
      vi.fn(() => true),
    );
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

const lspTool: AvailableTool = {
  id: "lsp",
  kind: "engine",
  origin: "workspace",
  permission_surface: "fixed",
};
const readTool: AvailableTool = {
  id: "read",
  kind: "core",
  origin: "builtin",
  permission_surface: "preset",
};
const mcpDemoTool: AvailableTool = {
  id: "mcp_demo",
  kind: "mcp",
  origin: "workspace",
  permission_surface: "fixed",
};

describe("AgentsSection capability bind cards", () => {
  beforeEach(() => {
    useSettingsStore.setState({
      llm: llmDoc,
      availableTools: [
        readTool,
        {
          id: "subagent_tools",
          kind: "core",
          origin: "builtin",
          permission_surface: "fixed",
          scope: "primary_root_only",
        },
      ],
      mcpDefs: { global: [], workspace: [] },
      mcpRuntime: { global: {}, workspace: {} },
      agentIds: ["default"],
      selectedAgentId: "default",
      agents: { default: profile() },
      persistByDoc: {},
      saveAgent: vi.fn(async () => undefined),
      createAgent: vi.fn(async () => undefined),
      removeAgent: vi.fn(async () => undefined),
      refreshAgents: vi.fn(async () => undefined),
    });
  });

  afterEach(() => {
    cleanup();
  });

  it("shows engine status without hiding the binding card", () => {
    useSettingsStore.setState({
      availableTools: [{ ...lspTool, status: "failed" }],
    });
    render(<AgentsSection />);
    expect(
      screen.getByRole("button", { name: /lsp tool binding, disabled/i }),
    ).toBeTruthy();
    expect(screen.getByText("failed")).toBeTruthy();
  });

  it("renders the subagent capability as one card", () => {
    render(<AgentsSection />);
    expect(
      screen.getAllByRole("button", { name: /subagent_tools tool binding/i }),
    ).toHaveLength(1);
    expect(
      screen.queryByRole("button", { name: /subagent_launch tool binding/i }),
    ).toBeNull();
  });
});

describe("AgentsSection tool binding sections", () => {
  beforeEach(() => {
    useSettingsStore.setState({
      llm: llmDoc,
      availableTools: [
        readTool,
        {
          id: "glob",
          kind: "core",
          origin: "builtin",
          permission_surface: "preset",
        },
        lspTool,
        {
          id: "demo",
          kind: "custom",
          origin: "global",
          permission_surface: "fixed",
        },
        mcpDemoTool,
      ],
      mcpDefs: { global: [], workspace: [] },
      mcpRuntime: { global: {}, workspace: {} },
      agentIds: ["default"],
      selectedAgentId: "default",
      agents: {
        default: profile({
          tools: { read: { enabled: true }, lsp: { enabled: true } },
        }),
      },
      persistByDoc: {},
      saveAgent: vi.fn(async () => undefined),
      createAgent: vi.fn(async () => undefined),
      removeAgent: vi.fn(async () => undefined),
      refreshAgents: vi.fn(async () => undefined),
    });
  });

  afterEach(() => {
    cleanup();
  });

  it("lays the cards out in kind sections with an on count", () => {
    render(<AgentsSection />);

    const headings = screen
      .getAllByRole("heading", { level: 4 })
      .map((node) => node.textContent);
    expect(headings).toEqual([
      "Core tools",
      "Engine tools",
      "Custom tools",
      "MCP servers",
    ]);
    // Core holds read + glob, one of them on; the rest report their own counts.
    expect(screen.getByText("1/2 on")).toBeTruthy();
    expect(screen.getByText("1/1 on")).toBeTruthy();
    expect(screen.getAllByText("0/1 on")).toHaveLength(2);
  });
});

describe("AgentsSection code-mode badge", () => {
  beforeEach(() => {
    useSettingsStore.setState({
      llm: llmDoc,
      availableTools: [
        { ...readTool, surface: "code" },
        {
          id: "glob",
          kind: "core",
          origin: "builtin",
          permission_surface: "preset",
          surface: "code",
        },
        {
          id: "grep",
          kind: "core",
          origin: "builtin",
          permission_surface: "preset",
          surface: "code",
        },
        {
          id: "edit",
          kind: "core",
          origin: "builtin",
          permission_surface: "preset",
          surface: "code",
        },
        {
          id: "write",
          kind: "core",
          origin: "builtin",
          permission_surface: "preset",
          surface: "code",
        },
        {
          id: "bash",
          kind: "core",
          origin: "builtin",
          permission_surface: "preset",
          surface: "direct",
        },
        {
          id: "demo",
          kind: "custom",
          origin: "global",
          permission_surface: "fixed",
          surface: "direct",
        },
        { ...mcpDemoTool, surface: "direct" },
        {
          id: "run_code",
          kind: "core",
          origin: "builtin",
          permission_surface: "fixed",
          surface: "direct",
        },
      ],
      mcpDefs: { global: [], workspace: [] },
      mcpRuntime: { global: {}, workspace: {} },
      agentIds: ["default"],
      selectedAgentId: "default",
      agents: { default: profile() },
      persistByDoc: {},
      saveAgent: vi.fn(async () => undefined),
      createAgent: vi.fn(async () => undefined),
      removeAgent: vi.fn(async () => undefined),
      refreshAgents: vi.fn(async () => undefined),
    });
  });

  afterEach(() => {
    cleanup();
  });

  it("badges code-surface tools and hides the run_code switch", () => {
    render(<AgentsSection />);
    for (const id of ["read", "glob", "grep", "edit", "write"]) {
      const toggle = screen.getByRole("button", {
        name: new RegExp(`^${id} tool binding`, "i"),
      });
      const card = toggle.closest(".tool-binding-card");
      expect(card).toBeTruthy();
      expect(within(card as HTMLElement).getByText("code mode")).toBeTruthy();
    }
    for (const id of ["bash", "demo", "mcp_demo"]) {
      const toggle = screen.getByRole("button", {
        name: new RegExp(`^${id} tool binding`, "i"),
      });
      const card = toggle.closest(".tool-binding-card");
      expect(card).toBeTruthy();
      expect(within(card as HTMLElement).queryByText("code mode")).toBeNull();
    }
    expect(
      screen.queryByRole("button", { name: /run_code tool binding/i }),
    ).toBeNull();
    expect(screen.getAllByText("code mode")).toHaveLength(5);
  });
});

describe("AgentsSection subagent tool cards", () => {
  const saveAgent = vi.fn(
    async (_id: string, _next: AgentProfile) => undefined,
  );
  const createAgent = vi.fn(async () => undefined);
  const removeAgent = vi.fn(async () => undefined);
  const refreshAgents = vi.fn(async () => undefined);

  beforeEach(() => {
    saveAgent.mockClear();
    useSettingsStore.setState({
      llm: llmDoc,
      availableTools: [
        readTool,
        lspTool,
        mcpDemoTool,
        {
          id: "plan",
          kind: "core",
          origin: "builtin",
          permission_surface: "fixed",
          scope: "primary_root_only",
        },
      ],
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

  it("hides primary-root-only cards from a subagent", () => {
    render(<AgentsSection />);
    expect(
      screen.getByRole("button", { name: /read tool binding/i }),
    ).toBeTruthy();
    expect(
      screen.queryByRole("button", { name: /plan tool binding/i }),
    ).toBeNull();
  });

  it("renders a suite label instead of its internal id", () => {
    useSettingsStore.setState({
      availableTools: [
        {
          id: "suite:deck",
          kind: "custom",
          origin: "global",
          permission_surface: "preset",
          label: "Deck",
        },
      ],
    });
    render(<AgentsSection />);
    expect(screen.getByText("Deck")).toBeTruthy();
    expect(screen.queryByText("suite:deck")).toBeNull();
  });

  it("renders per-tool preset cards with a deny-semantics note, not checkboxes", () => {
    render(<AgentsSection />);
    // Full card rows (clickable), one per bindable tool — not a checkbox list.
    expect(
      screen.getByRole("button", { name: /read tool binding, disabled/i }),
    ).toBeTruthy();
    expect(
      screen.getByRole("button", { name: /lsp tool binding, disabled/i }),
    ).toBeTruthy();
    expect(screen.queryByRole("checkbox")).toBeNull();
    // Configurable tools expose the ALL/SAFE preset control.
    const readPreset = screen.getByRole("group", { name: "read preset" });
    expect(
      within(readPreset).getByRole("button", { name: "SAFE" }),
    ).toBeTruthy();
    expect(
      within(readPreset).getByRole("button", { name: "ALL" }),
    ).toBeTruthy();
    expect(screen.queryByRole("group", { name: "lsp preset" })).toBeNull();
    expect(screen.getAllByText("Not configurable").length).toBeGreaterThan(0);
    // MCP server bindings expose the per-server tool visibility picker.
    expect(
      screen.getByRole("button", {
        name: "Select visible tools for MCP server demo",
      }),
    ).toBeTruthy();
    // Guidance on Ask -> deny semantics for subagent turns.
    expect(screen.getByText(/can't ask for approval/i)).toBeTruthy();
  });

  it("persists a SAFE preset picked on a subagent tool card", async () => {
    vi.useFakeTimers();
    render(<AgentsSection />);
    fireEvent.click(
      screen.getByRole("button", { name: /read tool binding, disabled/i }),
    );
    const readPreset = screen.getByRole("group", { name: "read preset" });
    fireEvent.click(within(readPreset).getByRole("button", { name: "SAFE" }));
    await vi.advanceTimersByTimeAsync(400);
    expect(saveAgent).toHaveBeenCalledTimes(1);
    const payload = saveAgent.mock.calls[0][1] as AgentProfile;
    expect(payload.tools.read).toEqual(
      expect.objectContaining({ enabled: true, preset: "SAFE" }),
    );
  });
});

describe("AgentsSection LSP bind persist loop", () => {
  const saveAgent = vi.fn(
    async (_id: string, _next: AgentProfile) => undefined,
  );

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
              preset: "ALL",
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
    let lastLsp: AgentToolBinding = {
      enabled: true,
      preset: "ALL",
    };
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
            lsp: { enabled: true, preset: "ALL" },
            mcp_other_ws: { enabled: true, preset: null },
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
            mcp_other_ws: { enabled: true, preset: null },
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
          preset: binding.preset ?? null,
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
      preset: null,
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

  it("marks each model option and the collapsed trigger with the provider's brand mark", () => {
    render(<AgentsSection />);

    // The trigger renders the selected option's label, so it carries the icon too.
    const trigger = screen.getByRole("button", { name: /^GPT$/ });
    expect(
      trigger.querySelector("[data-provider-logo]")?.getAttribute("title"),
    ).toBe("prov");

    const panel = openModelDropdown();
    // Same wire id, two providers — each row keeps its own owner's mark.
    const rows = [
      within(panel).getByRole("button", { name: /^GPT$/ }),
      within(panel).getByRole("button", { name: /Same Wire Id/ }),
    ];
    expect(
      rows.map((row) =>
        row.querySelector("[data-provider-logo]")?.getAttribute("title"),
      ),
    ).toEqual(["prov", "other"]);
  });

  it("groups active models by provider and keeps a duplicate wire id independently selectable", async () => {
    vi.useFakeTimers();
    render(<AgentsSection />);
    const panel = openModelDropdown();

    // Provider group headers, then one option per composite ref — both
    // providers expose the wire id `m1`, yet each keeps its own ref.
    expect(within(panel).getByText("Prov")).toBeTruthy();
    expect(within(panel).getByText("Other")).toBeTruthy();
    fireEvent.click(
      within(panel).getByRole("button", { name: /Same Wire Id/ }),
    );
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

    const trigger = screen.getByRole("button", {
      name: /Missing: ghost\/gone-model/,
    });
    fireEvent.click(trigger);
    const panel = document.querySelector(
      "[data-dropdown-panel]",
    ) as HTMLElement;
    const missing = within(panel).getAllByRole("button", {
      name: /Missing: ghost\/gone-model/,
    })[0];
    expect(missing.hasAttribute("disabled")).toBe(true);

    fireEvent.click(missing);
    await vi.advanceTimersByTimeAsync(400);
    // The ref is still the missing one — nothing was silently selected.
    expect(
      screen.getByRole("button", { name: /Missing: ghost\/gone-model/ }),
    ).toBeTruthy();
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

describe("AgentsSection first-run anchor", () => {
  const scrollIntoView = vi.fn();
  const saveAgent = vi.fn(async () => undefined);

  beforeEach(() => {
    saveAgent.mockClear();
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
      focusAnchor: "max-steps",
    });
    // jsdom has no scrollIntoView; the anchor effect must still call it.
    Element.prototype.scrollIntoView = scrollIntoView;
    // jsdom queues rAF callbacks but never runs them: make the detour synchronous.
    vi.spyOn(window, "requestAnimationFrame").mockImplementation(
      (cb: FrameRequestCallback) => {
        cb(0);
        return 1;
      },
    );
  });

  afterEach(() => {
    cleanup();
    scrollIntoView.mockClear();
    vi.restoreAllMocks();
    delete (Element.prototype as { scrollIntoView?: unknown }).scrollIntoView;
    useSettingsStore.setState({ focusAnchor: null });
  });

  it("centres the Max steps row, rings it and takes the caret", async () => {
    render(<AgentsSection />);

    await waitFor(() => {
      expect(scrollIntoView).toHaveBeenCalledWith({ block: "center" });
    });
    const row = document.querySelector('[data-anchor="max-steps"]');
    expect(row?.className).toContain("ring-(--_dk-amber-500)");
    expect(screen.getByText(/Long tasks may need ~200/)).toBeTruthy();
    expect(document.activeElement).toBe(row?.querySelector("input"));
    expect(useSettingsStore.getState().focusAnchor).toBeNull();
    // Guidance only: the 50 → 200 edit stays the user's.
    expect(saveAgent).not.toHaveBeenCalled();
  });

  it("consumes the anchor without flashing when another agent is selected", async () => {
    useSettingsStore.setState({
      agentIds: ["default", "other"],
      selectedAgentId: "other",
      agents: { default: profile(), other: profile({ description: "other" }) },
    });
    render(<AgentsSection />);

    await waitFor(() => {
      expect(useSettingsStore.getState().focusAnchor).toBeNull();
    });
    expect(screen.queryByText(/Long tasks may need ~200/)).toBeNull();
    expect(scrollIntoView).not.toHaveBeenCalled();
  });

  it("stays quiet without an anchor", () => {
    useSettingsStore.setState({ focusAnchor: null });
    render(<AgentsSection />);
    expect(screen.queryByText(/Long tasks may need ~200/)).toBeNull();
    expect(scrollIntoView).not.toHaveBeenCalled();
  });
});

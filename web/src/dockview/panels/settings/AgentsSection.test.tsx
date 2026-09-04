import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { AgentProfile, AgentToolBinding, AvailableTool, ModelDefinition } from "../../../api/settings";
import { useSettingsStore } from "../../../stores/settingsStore";
import { AgentsSection } from "./AgentsSection";

const model: ModelDefinition = {
  id: "m1",
  adapter_id: "openai",
  provider_ref: "prov",
  label: "GPT",
  config: {
    api_model_id: "gpt-4o",
    context_window: 200_000,
    max_tokens: 8192,
    capabilities: ["text"],
  },
};

function profile(patch: Partial<AgentProfile> = {}): AgentProfile {
  return {
    role: "primary",
    model_ref: "m1",
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
      models: { m1: model },
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

describe("AgentsSection LSP bind persist loop", () => {
  const saveAgent = vi.fn(async (_id: string, _next: AgentProfile) => undefined);

  beforeEach(() => {
    saveAgent.mockReset();
    useSettingsStore.setState({
      models: { m1: model },
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

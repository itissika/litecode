import {
  cleanup,
  fireEvent,
  render,
  screen,
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
    description: "",
    tools: {},
    allowed_subagents: [],
    builtin: true,
    ...patch,
  };
}

// Backend truth (W7): core cards are locked, optional/custom/MCP cards are not.
const lockedReadTool: AvailableTool = {
  id: "read",
  kind: "core",
  origin: "builtin",
  permission_surface: "preset",
  locked: true,
};
const lockedSubagentTool: AvailableTool = {
  id: "subagent_tools",
  kind: "core",
  origin: "builtin",
  permission_surface: "fixed",
  locked: true,
};
const lspTool: AvailableTool = {
  id: "lsp",
  kind: "engine",
  origin: "workspace",
  permission_surface: "fixed",
  locked: false,
};
const mcpDemoTool: AvailableTool = {
  id: "mcp_demo",
  kind: "mcp",
  origin: "workspace",
  permission_surface: "fixed",
  locked: false,
};

function seed(patch: {
  availableTools?: AvailableTool[];
  agents?: Record<string, AgentProfile>;
  agentIds?: string[];
  selectedAgentId?: string;
  saveAgent?: (id: string, next: AgentProfile) => Promise<void>;
  refreshAgents?: () => Promise<void>;
} = {}) {
  useSettingsStore.setState({
    llm: llmDoc,
    availableTools: patch.availableTools ?? [],
    mcpDefs: { global: [], workspace: [] },
    mcpRuntime: { global: {}, workspace: {} },
    agentIds: patch.agentIds ?? ["default"],
    selectedAgentId: patch.selectedAgentId ?? "default",
    agents: patch.agents ?? { default: profile() },
    persistByDoc: {},
    revision: 1,
    saveAgent: patch.saveAgent ?? vi.fn(async () => undefined),
    refreshAgents: patch.refreshAgents ?? vi.fn(async () => undefined),
  });
}

describe("AgentsSection read-only identity", () => {
  beforeEach(() => seed());
  afterEach(() => {
    cleanup();
    vi.useRealTimers();
  });

  it("renders no create, delete, role-switch, or identity-edit control", () => {
    render(<AgentsSection />);

    expect(screen.queryByRole("button", { name: "Add agent" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Delete" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Primary" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Subagent" })).toBeNull();
    expect(screen.queryByLabelText("System prompt")).toBeNull();
    expect(screen.queryAllByRole("textbox")).toHaveLength(0);
    expect(screen.queryAllByRole("checkbox")).toHaveLength(0);
  });

  it("shows the role in the agent picker and the description read-only", () => {
    seed({
      agents: {
        default: profile({
          description: "Default primary agent",
          allowed_subagents: ["general", "explore"],
        }),
      },
    });
    render(<AgentsSection />);

    // The role moved into the agent picker (icon + id + primary/subagent); the
    // old "Type" label is gone.
    const roleText = screen.getByText("Primary");
    expect(roleText.parentElement?.parentElement?.textContent).toContain(
      "default",
    );
    expect(screen.queryByText("Type")).toBeNull();
    // Description and allowed subagents stay visible, read-only.
    expect(screen.getByText("Default primary agent")).toBeTruthy();
    // One line only: the description truncates instead of wrapping, and it
    // sits on its own row above the allowed subagents.
    const description = document.querySelector("[data-agent-description]");
    expect(description?.className).toContain("truncate");
    expect(description?.parentElement?.className).toContain("flex-col");
    // Allowed subagents use the same chip shape (icon + text) as the role
    // badge, but as an unfilled frame.
    for (const id of ["general", "explore"]) {
      const chip = screen.getByText(id);
      expect(chip.className).toContain("tag-neutral");
      expect(chip.className).not.toContain("tag-soft");
      expect(chip.querySelector("svg")).toBeTruthy();
    }
  });

  it("lists primary agents before subagents, subagents dashed", () => {
    seed({
      agentIds: ["helper", "default"],
      agents: {
        default: profile(),
        helper: profile({ role: "subagent", description: "runs errands" }),
      },
    });
    render(<AgentsSection />);

    fireEvent.click(screen.getByRole("button", { name: /^default/ }));
    const panel = document.querySelector("[data-dropdown-panel]") as HTMLElement;
    const rows = within(panel).getAllByRole("button");
    // helper comes first in the store, but primaries sort to the top.
    expect(rows[0].textContent).toContain("default");
    expect(rows[0].textContent).toContain("--Primary");
    expect(rows[1].textContent).toContain("helper");
    expect(rows[1].textContent).toContain("--Subagent");
  });

  it("keeps the model picker editable and saves the chosen ref", async () => {
    vi.useFakeTimers();
    const saveAgent = vi.fn(async () => undefined);
    seed({
      saveAgent,
      agents: {
        default: profile({
          tools: { lsp: { enabled: true, preset: null } },
        }),
      },
      availableTools: [lspTool],
    });
    // `seed` restores the single-model doc; add the second model on top.
    const otherModel: CatalogModelDto = {
      ...model,
      ref: "other/m1",
      id: "m1",
      label: "Same Wire Id",
      provider_id: "other",
      provider_name: "Other",
    };
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
            models: [otherModel],
          },
        ],
        active_models: [model, otherModel],
      },
    });
    render(<AgentsSection />);

    fireEvent.click(screen.getByRole("button", { name: /^GPT$/ }));
    const panel = document.querySelector("[data-dropdown-panel]") as HTMLElement;
    expect(panel).toBeTruthy();
    fireEvent.click(within(panel).getByRole("button", { name: /Same Wire Id/ }));
    await vi.advanceTimersByTimeAsync(400);

    expect(saveAgent).toHaveBeenCalledWith(
      "default",
      expect.objectContaining({ model_ref: "other/m1" }),
    );
  });
});

describe("AgentsSection locked capability cards", () => {
  const saveAgent = vi.fn(async (_id: string, _next: AgentProfile) => undefined);

  beforeEach(() => {
    saveAgent.mockClear();
    seed({
      saveAgent,
      availableTools: [lockedReadTool, lspTool],
      agents: {
        default: profile({
          tools: {
            read: { enabled: true, preset: "SAFE" },
            lsp: { enabled: false },
          },
        }),
      },
    });
  });

  afterEach(() => {
    cleanup();
    vi.useRealTimers();
  });

  it("marks a locked card read-only and offers no working toggle", async () => {
    vi.useFakeTimers();
    render(<AgentsSection />);

    const toggle = screen.getByRole("button", {
      name: /^read tool binding, enabled/i,
    });
    const card = toggle.closest(".tool-binding-card") as HTMLElement;
    expect(card.getAttribute("data-locked")).toBe("true");
    expect(toggle).toHaveProperty("disabled", true);
    expect(toggle.getAttribute("aria-disabled")).toBe("true");
    // A locked card is a record, not a control: no on/off or lock tag.
    expect(within(card).queryByText("Locked")).toBeNull();
    expect(within(card).queryByText("On")).toBeNull();

    fireEvent.click(toggle);
    fireEvent.click(card);
    await vi.advanceTimersByTimeAsync(400);
    expect(saveAgent).not.toHaveBeenCalled();
  });

  it("shows the effective preset on a locked card but cannot change it", async () => {
    vi.useFakeTimers();
    render(<AgentsSection />);

    const preset = screen.getByRole("group", { name: "read preset" });
    const safe = within(preset).getByRole("button", { name: "SAFE" });
    const all = within(preset).getByRole("button", { name: "ALL" });
    expect(safe.getAttribute("aria-pressed")).toBe("true");
    expect(all.getAttribute("aria-pressed")).toBe("false");
    expect(safe).toHaveProperty("disabled", true);
    expect(all).toHaveProperty("disabled", true);

    fireEvent.click(all);
    await vi.advanceTimersByTimeAsync(400);
    expect(saveAgent).not.toHaveBeenCalled();
  });

  it("keeps a locked fixed-surface card's explanation instead of a preset", () => {
    seed({
      saveAgent,
      availableTools: [lockedSubagentTool],
      agents: {
        default: profile({ tools: { subagent_tools: { enabled: true } } }),
      },
    });
    render(<AgentsSection />);

    const toggle = screen.getByRole("button", {
      name: /^subagent_tools tool binding/i,
    });
    const card = toggle.closest(".tool-binding-card") as HTMLElement;
    expect(card.getAttribute("data-locked")).toBe("true");
    expect(within(card).queryByRole("group")).toBeNull();
    expect(within(card).getByText("Not configurable")).toBeTruthy();
  });

  it("leaves a non-locked card togglable and saves the new binding", async () => {
    vi.useFakeTimers();
    render(<AgentsSection />);

    const lsp = screen.getByRole("button", {
      name: /^lsp tool binding, disabled/i,
    });
    const lspCard = lsp.closest(".tool-binding-card") as HTMLElement;
    expect(lspCard.getAttribute("data-locked")).toBeNull();
    expect(lsp).toHaveProperty("disabled", false);

    fireEvent.click(lsp);
    await vi.advanceTimersByTimeAsync(400);
    expect(saveAgent).toHaveBeenCalledTimes(1);
    const saved = saveAgent.mock.calls[0][1] as AgentProfile;
    expect(saved.tools.lsp).toEqual(
      expect.objectContaining({ enabled: true }),
    );
    // The locked row rides along untouched.
    expect(saved.tools.read).toEqual({ enabled: true, preset: "SAFE" });
  });
});

describe("AgentsSection capability bind cards", () => {
  beforeEach(() => {
    seed({
      availableTools: [lockedReadTool, { ...lockedSubagentTool, scope: "primary_root_only" }],
    });
  });

  afterEach(() => {
    cleanup();
  });

  it("shows engine status without hiding the binding card", () => {
    seed({ availableTools: [{ ...lspTool, status: "failed" }] });
    render(<AgentsSection />);
    expect(
      screen.getByRole("button", { name: /lsp tool binding, disabled/i }),
    ).toBeTruthy();
    expect(screen.getByText("failed")).toBeTruthy();
  });

  it("renders the subagent capability as one card", () => {
    seed({
      availableTools: [lockedReadTool, { ...lockedSubagentTool, scope: "primary_root_only" }],
      agents: {
        default: profile({ tools: { subagent_tools: { enabled: true } } }),
      },
    });
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
    seed({
      availableTools: [
        lockedReadTool,
        {
          id: "glob",
          kind: "core",
          origin: "builtin",
          permission_surface: "preset",
          locked: true,
        },
        lspTool,
        {
          id: "demo",
          kind: "custom",
          origin: "global",
          permission_surface: "fixed",
          locked: false,
        },
        mcpDemoTool,
      ],
      agents: {
        default: profile({
          tools: { read: { enabled: true }, lsp: { enabled: true } },
        }),
      },
    });
  });

  afterEach(() => {
    cleanup();
  });

  it("folds every kind into a section, open by default, with an on count", () => {
    render(<AgentsSection />);

    const headers = [
      ...document.querySelectorAll<HTMLElement>(".foldcard-header"),
    ];
    // One header per section, in reading order, with its own count.
    expect(headers.map((node) => node.textContent)).toEqual([
      "Core tools 1/2 on",
      "Engine tools 1/1 on",
      "Custom tools 0/1 on",
      "MCP servers 0/1 on",
    ]);
    expect(headers.map((node) => node.getAttribute("aria-expanded"))).toEqual([
      "true",
      "true",
      "true",
      "true",
    ]);
    // Locked cards list only what is bound: read is on, glob is not.
    expect(
      screen.getByRole("button", { name: /^read tool binding/i }),
    ).toBeTruthy();
    expect(
      screen.queryByRole("button", { name: /^glob tool binding/i }),
    ).toBeNull();
    // Editable cards keep their off rows — that is where a bind happens.
    expect(
      screen.getByRole("button", { name: /^mcp_demo tool binding/i }),
    ).toBeTruthy();
  });
});

describe("AgentsSection hides run_code transport switch", () => {
  beforeEach(() => {
    seed({
      availableTools: [
        lockedReadTool,
        ...[
          "glob",
          "grep",
          "edit",
          "write",
          "bash",
        ].map(
          (id): AvailableTool => ({
            id,
            kind: "core",
            origin: "builtin",
            permission_surface: "preset",
            locked: true,
          }),
        ),
        {
          id: "demo",
          kind: "custom",
          origin: "global",
          permission_surface: "fixed",
          locked: false,
        },
        mcpDemoTool,
        {
          id: "run_code",
          kind: "core",
          origin: "builtin",
          permission_surface: "fixed",
          locked: true,
        },
      ],
      // Locked cards only render when bound, so bind the core rows this test
      // walks through.
      agents: {
        default: profile({
          tools: Object.fromEntries(
            ["read", "glob", "grep", "edit", "write", "bash"].map((id) => [
              id,
              { enabled: true },
            ]),
          ),
        }),
      },
    });
  });

  afterEach(() => {
    cleanup();
  });

  it("renders tool cards without code-mode badges and hides run_code", () => {
    render(<AgentsSection />);
    for (const id of ["read", "glob", "grep", "edit", "write", "bash", "demo", "mcp_demo"]) {
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
  });
});

describe("AgentsSection subagent tool cards", () => {
  const saveAgent = vi.fn(
    async (_id: string, _next: AgentProfile) => undefined,
  );

  beforeEach(() => {
    saveAgent.mockClear();
    seed({
      saveAgent,
      availableTools: [
        { ...lockedReadTool, locked: false },
        lspTool,
        mcpDemoTool,
        {
          id: "plan",
          kind: "core",
          origin: "builtin",
          permission_surface: "fixed",
          locked: true,
          scope: "primary_root_only",
        },
      ],
      agentIds: ["helper"],
      selectedAgentId: "helper",
      agents: { helper: profile({ role: "subagent" }) },
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
    seed({
      saveAgent,
      availableTools: [
        {
          id: "suite:deck",
          kind: "custom",
          origin: "global",
          permission_surface: "preset",
          label: "Deck",
          locked: false,
        },
      ],
      agentIds: ["helper"],
      selectedAgentId: "helper",
      agents: { helper: profile({ role: "subagent" }) },
    });
    render(<AgentsSection />);
    expect(screen.getByText("Deck")).toBeTruthy();
    expect(screen.queryByText("suite:deck")).toBeNull();
  });

  it("renders per-tool preset cards, not checkboxes", () => {
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
  });

  it("persists a SAFE preset picked on an unlocked subagent tool card", async () => {
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
    seed({ saveAgent, availableTools: [lockedReadTool, lspTool] });
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
      availableTools: [lockedReadTool, lspTool],
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
          availableTools: lspListed
            ? [lockedReadTool, lspTool]
            : [lockedReadTool],
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
    seed({
      saveAgent,
      availableTools: [lockedReadTool, lspTool],
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
    seed({
      saveAgent,
      availableTools: [lockedReadTool, lspTool],
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

  it("tells the user to configure a provider API key when no model is active", () => {
    useSettingsStore.setState({
      llm: { ...llmDoc, active_models: [] },
    });
    render(<AgentsSection />);
    expect(screen.getByText("Configure a provider API key")).toBeTruthy();
  });
});

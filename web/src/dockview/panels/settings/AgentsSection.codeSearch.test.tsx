// WP-150 / WP-145: code_search bind card reflects only the stored binding.
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type {
  AgentProfile,
  AvailableTool,
  CatalogModelDto,
  EngineWarmupState,
  LlmSettings,
} from "../../../api/settings";
import { applyToolEnabled, putAgent } from "../../../api/settings";
import { useEngineStore } from "../../../stores/engineStore";
import { useSettingsStore } from "../../../stores/settingsStore";
import { AgentsSection } from "./AgentsSection";

const model: CatalogModelDto = {
  ref: "prov/m1", id: "gpt-4o", label: "GPT", provider_id: "prov",
  provider_name: "Prov", context_window: 200_000, context_window_max: 400_000,
  modalities: ["text"], tool_call: true, json_output: false, enabled: true,
};
const llmDoc: LlmSettings = {
  catalog_path: "x", revision: 1,
  providers: [{
    id: "prov", name: "Prov", visible: true, configured: true,
    masked_api_key: "sk-***x", endpoint: "https://prov.example/v1",
    endpoint_type: "responses", models: [model],
  }],
  active_models: [model],
};

const codeSearchTool: AvailableTool = {
  id: "code_search", kind: "engine", origin: "workspace", permission_surface: "fixed",
};
const readTool: AvailableTool = {
  id: "read", kind: "core", origin: "builtin", permission_surface: "preset",
};
const sessionSearchTool: AvailableTool = {
  id: "session_search", kind: "engine", origin: "workspace", permission_surface: "fixed",
};

function profile(patch: Partial<AgentProfile> = {}): AgentProfile {
  return {
    role: "primary", model_ref: "prov/m1", system_prompt: "", temperature: 0.7,
    max_steps: 50, description: "", tools: {}, allowed_subagents: [], ...patch,
  };
}

const card = (state: "enabled" | "disabled") =>
  screen.getByRole("button", {
    name: new RegExp(`^code_search tool binding, ${state}`, "i"),
  });

function setEngine(state: EngineWarmupState) {
  useEngineStore.setState({
    engineStatuses: { code_search: { desired: true, state, error: null } },
  });
}

describe("AgentsSection code_search binding (WP-150)", () => {
  const saveAgent = vi.fn(async (_id: string, _next: AgentProfile) => undefined);
  const createAgent = vi.fn(async (_id: string, _next: AgentProfile) => undefined);

  function seed(agent: AgentProfile, available: AvailableTool[] | null = [readTool, codeSearchTool, sessionSearchTool]) {
    useSettingsStore.setState({
      llm: llmDoc,
      availableTools: available,
      mcpDefs: { global: [], workspace: [] },
      mcpRuntime: { global: {}, workspace: {} },
      agentIds: ["a"],
      selectedAgentId: "a",
      agents: { a: agent },
      persistByDoc: {},
      saveAgent,
      createAgent,
      removeAgent: vi.fn(async () => undefined),
      refreshAgents: vi.fn(async () => undefined),
    });
  }

  beforeEach(() => {
    saveAgent.mockClear();
    createAgent.mockClear();
  });
  afterEach(() => {
    cleanup();
    vi.useRealTimers();
    vi.unstubAllGlobals();
    useEngineStore.setState({ engineStatuses: {} });
  });

  for (const state of ["stopped", "warming"] as const) {
    it(`shows stored On while engine is ${state}, not greyed, togglable`, async () => {
      setEngine(state);
      seed(profile({ tools: { code_search: { enabled: true, preset: "ALL" } } }));
      vi.useFakeTimers();
      render(<AgentsSection />);
      const btn = card("enabled");
      expect(btn.getAttribute("aria-pressed")).toBe("true");
      expect((btn as HTMLButtonElement).disabled).toBe(false);
      fireEvent.click(btn);
      expect(card("disabled")).toBeTruthy();
      await vi.advanceTimersByTimeAsync(400);
      expect(saveAgent).toHaveBeenCalled();
      const saved = saveAgent.mock.calls.at(-1)![1];
      expect(saved.tools.code_search?.enabled).toBe(false);
    });

    it(`does not rewrite stored code_search on save while engine is ${state}`, async () => {
      setEngine(state);
      seed(profile({
        tools: {
          code_search: { enabled: true, preset: "ALL" },
          mcp_other_ws: { enabled: true, preset: "ALL" },
        },
      }), [readTool, codeSearchTool]);
      vi.useFakeTimers();
      render(<AgentsSection />);
      fireEvent.click(screen.getByRole("button", { name: /^read tool binding/i }));
      await vi.advanceTimersByTimeAsync(400);
      const saved = saveAgent.mock.calls.at(-1)![1];
      expect(saved.tools.code_search).toEqual({ enabled: true, preset: "ALL" });
      expect(saved.tools.mcp_other_ws).toEqual({ enabled: true, preset: "ALL" });
    });
  }

  for (const state of ["stopped", "warming"] as const) {
    it(`session_search card unaffected while engine is ${state}`, () => {
      setEngine(state);
      seed(profile({
        tools: {
          code_search: { enabled: true, preset: "ALL" },
          session_search: { enabled: true, preset: "ALL" },
        },
      }));
      render(<AgentsSection />);
      const ss = screen.getByRole("button", { name: /^session_search tool binding, enabled/i });
      expect(ss.getAttribute("aria-pressed")).toBe("true");
      expect((ss as HTMLButtonElement).disabled).toBe(false);
      cleanup();
      seed(profile({ tools: { session_search: { enabled: false, preset: "ALL" } } }));
      render(<AgentsSection />);
      const off = screen.getByRole("button", { name: /^session_search tool binding, disabled/i });
      expect(off.getAttribute("aria-pressed")).toBe("false");
    });
  }

  it("primary agent: missing code_search row renders Off", () => {
    seed(profile());
    render(<AgentsSection />);
    expect(card("disabled").getAttribute("aria-pressed")).toBe("false");
  });

  it("subagent grid shows catalog code_search; missing row renders Off", () => {
    seed(profile({ role: "subagent" }));
    render(<AgentsSection />);
    expect(card("disabled")).toBeTruthy();
    expect(card("disabled").getAttribute("aria-pressed")).toBe("false");
  });

  it("subagent grid shows stored On", () => {
    seed(profile({ role: "subagent", tools: { code_search: { enabled: true, preset: "ALL" } } }));
    render(<AgentsSection />);
    expect(card("enabled")).toBeTruthy();
  });

  it("renders no code_search card before the catalog has loaded", () => {
    seed(profile(), null);
    render(<AgentsSection />);
    expect(screen.queryByRole("button", { name: /^code_search tool binding/i })).toBeNull();
  });

  it("new agent payload does not write code_search", async () => {
    seed(profile());
    render(<AgentsSection />);
    fireEvent.click(screen.getByRole("button", { name: "Add agent" }));
    fireEvent.change(screen.getByPlaceholderText("my_agent"), { target: { value: "my_agent" } });
    fireEvent.click(screen.getByRole("button", { name: "Create" }));
    await waitFor(() => expect(createAgent).toHaveBeenCalled());
    const payload = createAgent.mock.calls[0][1];
    expect("code_search" in payload.tools).toBe(false);
  });
});

describe("putAgent keeps code_search (WP-150)", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("PUT body carries full tools including code_search and out-of-catalog keys", async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true, status: 200, json: async () => ({ ok: true, revision: 2, docs: ["agents"] }),
    });
    vi.stubGlobal("fetch", fetchMock);
    const tools = {
      code_search: { enabled: true, preset: "ALL" as const },
      mcp_other_ws: { enabled: false, preset: "ALL" as const },
    };
    await putAgent("a", profile({ tools }));
    const [, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    const body = JSON.parse(init.body as string) as AgentProfile;
    expect(body.tools.code_search).toEqual(tools.code_search);
    expect(body.tools.mcp_other_ws).toEqual(tools.mcp_other_ws);
  });

  it("toggling code_search writes only its own row", () => {
    const on = applyToolEnabled(
      { code_search: { enabled: true, preset: "ALL" } },
      "code_search",
      false,
    );
    expect(on.code_search.enabled).toBe(false);
    expect("code_search" in applyToolEnabled({}, "code_search", true)).toBe(true);
  });
});

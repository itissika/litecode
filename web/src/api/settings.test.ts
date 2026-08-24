import { describe, expect, it, vi, afterEach } from "vitest";

import {
  SettingsApiError,
  isAgentVisible,
  isConfigurableTool,
  putLog,
} from "./settings";
import type { AvailableTool } from "./settings";

describe("settings helpers", () => {
  it("identifies NONE tools without preset", () => {
    expect(isConfigurableTool("read")).toBe(true);
    expect(isConfigurableTool("plan")).toBe(false);
    expect(isConfigurableTool("subagent_launch")).toBe(false);
    expect(isConfigurableTool("mcp_github")).toBe(false);
    expect(isConfigurableTool("echo_py")).toBe(true);
  });

  it("filters visible agent roles", () => {
    expect(isAgentVisible("primary")).toBe(true);
    expect(isAgentVisible("subagent")).toBe(true);
    expect(isAgentVisible("hidden")).toBe(true);
  });

  it("identifies compaction as configurable hidden agent", async () => {
    const { isHiddenSettingsAgent } = await import("./settings");
    expect(isHiddenSettingsAgent("compaction", "hidden")).toBe(true);
    expect(isHiddenSettingsAgent("other", "hidden")).toBe(false);
  });

  it("model option label prefers label field", async () => {
    const { modelOptionLabel } = await import("./settings");
    expect(
      modelOptionLabel({
        id: "model_123",
        adapter_id: "openai_responses",
        provider_ref: "default",
        label: "Sonnet",
        config: {
          api_model_id: "x",
          context_window: 1,
          max_tokens: 1,
          capabilities: ["text"],
        },
      }),
    ).toBe("Sonnet");
  });

  it("identifies subagent bindable tools excluding subagent series", async () => {
    const { isSubagentBindableTool, SUBAGENT_SERIES_TOOL_IDS } = await import("./settings");
    const webfetch: AvailableTool = {
      id: "webfetch",
      kind: "core",
      origin: "builtin",
    };
    const launch: AvailableTool = {
      id: "subagent_launch",
      kind: "core",
      origin: "builtin",
    };
    expect(isSubagentBindableTool(webfetch)).toBe(true);
    expect(isSubagentBindableTool(launch)).toBe(false);
    expect(SUBAGENT_SERIES_TOOL_IDS.has("subagent_launch")).toBe(true);
  });

  it("links bash wait_shell kill_shell as one enable series", async () => {
    const {
      BASH_SERIES_TOOL_IDS,
      applyToolEnabled,
      syncToolEnableSeries,
      toolEnableSeries,
      withSyncedToolSeries,
    } = await import("./settings");
    expect([...BASH_SERIES_TOOL_IDS]).toEqual(["bash", "wait_shell", "kill_shell"]);
    expect(toolEnableSeries("wait_shell")).toEqual(["bash", "wait_shell", "kill_shell"]);
    expect(toolEnableSeries("read")).toBeNull();

    const enabled = applyToolEnabled({}, "kill_shell", true);
    expect(enabled.bash.enabled).toBe(true);
    expect(enabled.wait_shell.enabled).toBe(true);
    expect(enabled.kill_shell.enabled).toBe(true);

    const disabled = applyToolEnabled(enabled, "bash", false);
    expect(disabled.bash.enabled).toBe(false);
    expect(disabled.wait_shell.enabled).toBe(false);
    expect(disabled.kill_shell.enabled).toBe(false);

    const mixed = syncToolEnableSeries({
      bash: { enabled: true, last_applied_preset: "ALL" },
      read: { enabled: true, last_applied_preset: "SAFE" },
    });
    expect(mixed.bash.enabled).toBe(true);
    expect(mixed.wait_shell.enabled).toBe(true);
    expect(mixed.kill_shell.enabled).toBe(true);
    expect(mixed.read.enabled).toBe(true);
    expect(mixed.read.last_applied_preset).toBe("SAFE");

    const profile = withSyncedToolSeries({
      role: "primary",
      model_ref: "",
      system_prompt: "",
      temperature: 0.7,
      max_steps: 50,
      description: "",
      allowed_subagents: [],
      tools: { bash: { enabled: true, last_applied_preset: "ALL" } },
    });
    expect(profile.tools.wait_shell?.enabled).toBe(true);
    expect(profile.tools.kill_shell?.enabled).toBe(true);
  });

  it("identifies protected agents", async () => {
    const { isProtectedAgent } = await import("./settings");
    expect(isProtectedAgent("default")).toBe(true);
    expect(isProtectedAgent("compaction")).toBe(true);
    expect(isProtectedAgent("reviewer")).toBe(false);
  });

});

describe("settings API response parsing", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("unwraps flattened ok payloads (no data wrapper)", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue({
        ok: true,
        status: 200,
        json: async () => ({
          ok: true,
          models: {
            default: {
              id: "default",
              api_model_id: "m",
              context_window: 1,
              max_tokens: 1,
              label: "Default",
            },
          },
        }),
      }),
    );

    const { getModels } = await import("./settings");
    const models = await getModels();
    expect(models.default?.id).toBe("default");
  });
});

describe("settings API errors", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("maps 409 turn_in_progress", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue({
        ok: false,
        status: 409,
        json: async () => ({ ok: false, error: "turn_in_progress" }),
      }),
    );

    await expect(putLog("debug")).rejects.toSatisfy((err: unknown) => {
      expect(err).toBeInstanceOf(SettingsApiError);
      const apiErr = err as SettingsApiError;
      expect(apiErr.isTurnBlocked).toBe(true);
      expect(apiErr.status).toBe(409);
      return true;
    });
  });
});

describe("settings_changed wire envelope", () => {
  it("parses settings_changed frame", () => {
    const env = {
      settings_changed: {
        revision: 3,
        summary: {
          revision: 3,
          provider_endpoint: "http://x",
          model_count: 1,
          agent_count: 2,
          catalog_count: 10,
          log_level: "info",
          effective_next_turn: true,
          restart_required: false,
        },
      },
    };
    expect(env).not.toBeNull();
    expect("settings_changed" in env).toBe(true);
    expect(env.settings_changed.revision).toBe(3);
  });
});

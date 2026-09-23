import { describe, expect, it, vi, afterEach } from "vitest";

import {
  SettingsApiError,
  isAgentVisible,
  isConfigurableTool,
  putLog,
} from "./settings";
import type { AvailableTool, CatalogModelDto } from "./settings";

function catalogModel(patch: Partial<CatalogModelDto> = {}): CatalogModelDto {
  return {
    ref: "commandcode/deepseek/deepseek-v4-flash",
    id: "deepseek/deepseek-v4-flash",
    label: "DeepSeek V4 Flash",
    provider_id: "commandcode",
    provider_name: "Command Code",
    context_window: 200_000,
    context_window_max: 400_000,
    modalities: ["text"],
    tool_call: true,
    json_output: false,
    enabled: true,
    ...patch,
  };
}

describe("settings helpers", () => {
  it("identifies NONE tools without preset", () => {
    expect(isConfigurableTool("read")).toBe(true);
    expect(isConfigurableTool("plan")).toBe(false);
    expect(isConfigurableTool("subagent_launch")).toBe(false);
    expect(isConfigurableTool("subagent_wait")).toBe(false);
    expect(isConfigurableTool("subagent_stop")).toBe(false);
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

  it("labels a catalog model from its label, falling back to the wire id", async () => {
    const { modelRefLabel } = await import("./settings");
    expect(modelRefLabel({ ...catalogModel(), label: "Sonnet" })).toBe("Sonnet");
    expect(modelRefLabel({ ...catalogModel(), label: "" })).toBe(
      "deepseek/deepseek-v4-flash",
    );
  });

  it("splits a composite ref on the first / only — model ids may contain /", async () => {
    const { splitModelRef } = await import("./settings");
    expect(splitModelRef("openai/gpt-5.4")).toEqual({
      providerId: "openai",
      modelId: "gpt-5.4",
    });
    expect(splitModelRef("commandcode/deepseek/deepseek-v4-flash")).toEqual({
      providerId: "commandcode",
      modelId: "deepseek/deepseek-v4-flash",
    });
    // No separator: keep the whole thing as the model id (never guess).
    expect(splitModelRef("bare-model")).toEqual({
      providerId: "",
      modelId: "bare-model",
    });
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
    const plan: AvailableTool = {
      id: "plan",
      kind: "core",
      origin: "builtin",
    };
    expect(isSubagentBindableTool(webfetch)).toBe(true);
    expect(isSubagentBindableTool(launch)).toBe(false);
    expect(isSubagentBindableTool(plan)).toBe(false);
    expect((SUBAGENT_SERIES_TOOL_IDS as readonly string[]).includes("subagent_launch")).toBe(true);
    expect((SUBAGENT_SERIES_TOOL_IDS as readonly string[]).includes("subagent_wait")).toBe(true);
    expect((SUBAGENT_SERIES_TOOL_IDS as readonly string[]).includes("subagent_stop")).toBe(true);
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

  it("links subagent launch wait stop as one enable series", async () => {
    const {
      SUBAGENT_SERIES_TOOL_IDS,
      applyToolEnabled,
      syncToolEnableSeries,
      toolEnableSeries,
    } = await import("./settings");
    expect([...SUBAGENT_SERIES_TOOL_IDS]).toEqual([
      "subagent_launch",
      "subagent_wait",
      "subagent_stop",
      "subagent_list",
      "subagent_send",
    ]);
    expect(toolEnableSeries("subagent_wait")).toEqual([
      "subagent_launch",
      "subagent_wait",
      "subagent_stop",
      "subagent_list",
      "subagent_send",
    ]);

    const enabled = applyToolEnabled({}, "subagent_launch", true);
    expect(enabled.subagent_launch.enabled).toBe(true);
    expect(enabled.subagent_wait.enabled).toBe(true);
    expect(enabled.subagent_stop.enabled).toBe(true);
    expect(enabled.subagent_list.enabled).toBe(true);
    expect(enabled.subagent_send.enabled).toBe(true);

    const disabled = applyToolEnabled(enabled, "subagent_stop", false);
    expect(disabled.subagent_launch.enabled).toBe(false);
    expect(disabled.subagent_wait.enabled).toBe(false);
    expect(disabled.subagent_stop.enabled).toBe(false);
    expect(disabled.subagent_list.enabled).toBe(false);
    expect(disabled.subagent_send.enabled).toBe(false);

    const mixed = syncToolEnableSeries({
      subagent_launch: { enabled: true, last_applied_preset: null },
    });
    expect(mixed.subagent_launch.enabled).toBe(true);
    expect(mixed.subagent_wait.enabled).toBe(true);
    expect(mixed.subagent_stop.enabled).toBe(true);
    expect(mixed.subagent_list.enabled).toBe(true);
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

  it("unwraps the flattened llm document (no data wrapper)", async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => ({
        ok: true,
        catalog_path: "C:\\Users\\x\\provider-catalog.toml",
        revision: 7,
        providers: [
          {
            id: "openai",
            name: "OpenAI",
            visible: true,
            configured: true,
            masked_api_key: "sk-***abcd",
            endpoint: "https://api.openai.com/v1",
            endpoint_type: "responses",
            models: [],
          },
        ],
        active_models: [catalogModel()],
      }),
    });
    vi.stubGlobal("fetch", fetchMock);

    const { getLlmSettings } = await import("./settings");
    const llm = await getLlmSettings();
    expect(fetchMock.mock.calls[0]?.[0]).toBe("/api/settings/llm");
    expect(llm.revision).toBe(7);
    expect(llm.catalog_path).toContain("provider-catalog.toml");
    expect(llm.providers[0]?.configured).toBe(true);
    expect(llm.active_models[0]?.ref).toBe(
      "commandcode/deepseek/deepseek-v4-flash",
    );
  });
});

describe("provider credential API", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("PUTs only the provider key endpoint", async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => ({ ok: true, revision: 8, docs: ["llm"] }),
    });
    vi.stubGlobal("fetch", fetchMock);

    const { putProviderKey } = await import("./settings");
    const result = await putProviderKey("opencode-go", "sk-test");
    expect(result.revision).toBe(8);
    expect(fetchMock).toHaveBeenCalledTimes(1);
    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    // Provider ids with /`/` survive path encoding.
    expect(url).toBe("/api/settings/providers/opencode-go/key");
    expect(init.method).toBe("PUT");
    expect(JSON.parse(init.body as string)).toEqual({ api_key: "sk-test" });
  });

  it("DELETEs the provider key endpoint", async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => ({ ok: true, revision: 9, docs: ["llm"] }),
    });
    vi.stubGlobal("fetch", fetchMock);

    const { deleteProviderKey } = await import("./settings");
    const result = await deleteProviderKey("ark-coding");
    expect(result.revision).toBe(9);
    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(url).toBe("/api/settings/providers/ark-coding/key");
    expect(init.method).toBe("DELETE");
  });

  it("surfaces an unknown provider as a 404 SettingsApiError", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue({
        ok: false,
        status: 404,
        json: async () => ({ ok: false, error: "unknown_provider" }),
      }),
    );

    const { putProviderKey } = await import("./settings");
    await expect(putProviderKey("nope", "sk-x")).rejects.toSatisfy(
      (err: unknown) => {
        expect(err).toBeInstanceOf(SettingsApiError);
        expect((err as SettingsApiError).status).toBe(404);
        expect((err as SettingsApiError).code).toBe("unknown_provider");
        return true;
      },
    );
  });

  it("surfaces an empty key as a 400 SettingsApiError", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue({
        ok: false,
        status: 400,
        json: async () => ({ ok: false, error: "empty_api_key" }),
      }),
    );

    const { putProviderKey } = await import("./settings");
    await expect(putProviderKey("openai", "")).rejects.toSatisfy(
      (err: unknown) => {
        expect((err as SettingsApiError).status).toBe(400);
        return true;
      },
    );
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
        docs: ["llm"],
        summary: {
          revision: 3,
          configured_provider_count: 2,
          active_model_count: 17,
          agent_count: 2,
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

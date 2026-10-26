import { describe, expect, it, vi, afterEach } from "vitest";

import {
  SettingsApiError,
  isCodeSurface,
  isConfigurableTool,
  putLog,
  settingsToolSwitches,
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
  it("badges only a declared code surface and drops the transport switch", () => {
    expect(isCodeSurface({ surface: "code" })).toBe(true);
    expect(isCodeSurface({ surface: "direct" })).toBe(false);
    expect(isCodeSurface({})).toBe(false);
    const tools = settingsToolSwitches([
      { id: "read", kind: "core", origin: "builtin", surface: "code" },
      { id: "bash", kind: "core", origin: "builtin", surface: "direct" },
      { id: "run_code", kind: "core", origin: "builtin" },
    ]);
    expect(tools.map((tool) => tool.id)).toEqual(["read", "bash"]);
  });

  it("shows ALL/SAFE only when the backend card says preset", () => {
    expect(isConfigurableTool({ permission_surface: "preset" })).toBe(true);
    expect(isConfigurableTool({ permission_surface: "fixed" })).toBe(false);
    expect(isConfigurableTool({})).toBe(false);
    const subagentCapability = {
      id: "subagent_tools",
      permission_surface: "fixed" as const,
    };
    expect(isConfigurableTool(subagentCapability)).toBe(false);
  });

  it("labels a catalog model from its label, falling back to the wire id", async () => {
    const { modelRefLabel } = await import("./settings");
    expect(modelRefLabel({ ...catalogModel(), label: "Sonnet" })).toBe(
      "Sonnet",
    );
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

  it("uses backend scope and leaves the subagent capability as one row", async () => {
    const { applyToolEnabled } = await import("./settings");
    const subagent: AvailableTool = {
      id: "subagent_tools",
      kind: "core",
      origin: "builtin",
      permission_surface: "fixed",
      scope: "primary_root_only",
      members: ["subagent_launch", "subagent_wait"],
    };
    const read: AvailableTool = {
      id: "read",
      kind: "core",
      origin: "builtin",
      scope: "all",
    };
    expect(subagent.scope).toBe("primary_root_only");
    expect(read.scope).toBe("all");

    const enabled = applyToolEnabled({}, "subagent_tools", true);
    expect(enabled.subagent_tools.enabled).toBe(true);
    expect(Object.keys(enabled)).toEqual(["subagent_tools"]);
  });

  it("one suite card writes one suite binding row", async () => {
    const { applyToolEnabled } = await import("./settings");
    const suite: AvailableTool = {
      id: "suite:deck",
      kind: "custom",
      origin: "global",
      label: "Deck",
      members: ["suite_a", "suite_b"],
    };
    expect(suite.members).toEqual(["suite_a", "suite_b"]);
    expect(suite.label).toBe("Deck");
    const enabled = applyToolEnabled({}, "suite:deck", true);
    expect(enabled["suite:deck"].enabled).toBe(true);
    expect(Object.keys(enabled)).toEqual(["suite:deck"]);
  });

  it("identifies protected agents", async () => {
    const { isProtectedAgent } = await import("./settings");
    expect(isProtectedAgent("default")).toBe(true);
    expect(isProtectedAgent("compaction")).toBe(false);
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

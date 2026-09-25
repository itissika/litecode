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
  CatalogModelDto,
  CatalogProviderDto,
  LlmSettings,
} from "../../../api/settings";
import { useSettingsStore } from "../../../stores/settingsStore";
import { ModelsSection } from "./ModelsSection";

vi.mock("../../../api/settings", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../../api/settings")>();
  return {
    ...actual,
    getLlmSettings: vi.fn(),
    putModelEnabled: vi.fn(),
  };
});

import { getLlmSettings, putModelEnabled } from "../../../api/settings";

const mockedLlm = vi.mocked(getLlmSettings);
const mockedEnabled = vi.mocked(putModelEnabled);

function model(patch: Partial<CatalogModelDto> = {}): CatalogModelDto {
  return {
    ref: "opencode/go-1",
    id: "go-1",
    label: "Go One",
    provider_id: "opencode",
    provider_name: "OpenCode Zen",
    context_window: 200_000,
    context_window_max: 400_000,
    modalities: ["text"],
    tool_call: true,
    json_output: false,
    enabled: true,
    ...patch,
  };
}

function modelProvider(
  patch: Partial<CatalogProviderDto> = {},
): CatalogProviderDto {
  return {
    id: "opencode",
    name: "OpenCode Zen",
    visible: true,
    configured: true,
    masked_api_key: "sk-***zen",
    endpoint: "https://opencode.ai/zen/v1",
    endpoint_type: "chat_completions",
    models: [],
    ...patch,
  };
}

const solModel = model({
  ref: "opencode/gpt-6-sol",
  id: "gpt-6-sol",
  label: "GPT-6 Sol",
});
const offModel = model({
  ref: "opencode/kimi-k2",
  id: "kimi-k2",
  label: "Kimi K2",
  enabled: false,
});

const zenProvider = modelProvider({
  id: "opencode",
  name: "OpenCode Zen",
  models: [solModel, offModel],
});
const goProvider = modelProvider({
  id: "opencode-go",
  name: "OpenCode Go",
  models: [
    model({ ref: "opencode-go/deepseek-v4-flash", label: "DeepSeek V4 Flash" }),
  ],
});

const unconfigured = modelProvider({
  id: "ark-coding",
  name: "Ark Coding",
  configured: false,
  masked_api_key: null,
  models: [model({ ref: "ark-coding/x", label: "Doubao" })],
});

function llmDoc(patch: Partial<LlmSettings> = {}): LlmSettings {
  return {
    catalog_path: "C:\\Users\\x\\provider-catalog.toml",
    revision: 3,
    providers: [zenProvider, goProvider, unconfigured],
    active_models: [solModel],
    ...patch,
  };
}

/** The segmented On/Off picker for one model row. */
function toggleFor(modelLabel: string): HTMLElement {
  return screen.getByRole("group", { name: `Enable ${modelLabel}` });
}

/** One modality glyph inside a model row. */
function modalityIcon(
  modelLabel: string,
  modality: string,
): HTMLElement | null {
  const row = screen.getByText(modelLabel).closest('[role="listitem"]');
  return row?.querySelector(`[role="img"][aria-label="${modality}"]`) ?? null;
}

/** The section renders the store's `llm`, so a test seeds it through setState. */
function seedLlm(doc: LlmSettings) {
  useSettingsStore.setState({ llm: doc });
}

describe("ModelsSection", () => {
  beforeEach(() => {
    mockedLlm.mockReset().mockResolvedValue(llmDoc());
    mockedEnabled.mockReset().mockResolvedValue({ revision: 4, docs: ["llm"] });
    useSettingsStore.setState({
      open: true,
      section: "models",
      revision: 3,
      llm: llmDoc(),
      persistByDoc: {},
      docClock: {},
      loadError: null,
    });
  });

  afterEach(() => {
    cleanup();
  });

  it("shows one card per provider that holds a key, and hides the rest", () => {
    render(<ModelsSection />);

    expect(toggleFor("GPT-6 Sol")).toBeTruthy();
    expect(toggleFor("DeepSeek V4 Flash")).toBeTruthy();
    // No credential → no usable models → absent from this page.
    expect(screen.queryByRole("group", { name: "Enable Doubao" })).toBeNull();
  });

  it("renders a bare list: name plus switch, no catalog badges", () => {
    render(<ModelsSection />);

    expect(screen.getByText("GPT-6 Sol")).toBeTruthy();
    // The tags the Provider page used to carry are gone.
    expect(screen.queryByText("200k")).toBeNull();
    expect(screen.queryByText("opencode/gpt-6-sol")).toBeNull();
    // `["text"]` alone earns no glyph: the row stays name plus switch.
    expect(modalityIcon("GPT-6 Sol", "text")).toBeNull();
    expect(modalityIcon("GPT-6 Sol", "image")).toBeNull();
  });

  it("reads each switch straight off the projection", () => {
    render(<ModelsSection />);

    expect(
      within(toggleFor("GPT-6 Sol"))
        .getByRole("button", { name: "On" })
        .getAttribute("aria-pressed"),
    ).toBe("true");
    expect(
      within(toggleFor("GPT-6 Sol"))
        .getByRole("button", { name: "Off" })
        .getAttribute("aria-pressed"),
    ).toBe("false");
    expect(
      within(toggleFor("Kimi K2"))
        .getByRole("button", { name: "On" })
        .getAttribute("aria-pressed"),
    ).toBe("false");
  });

  it("switches a model off through the model API and refetches", async () => {
    mockedLlm.mockResolvedValue(
      llmDoc({
        revision: 4,
        providers: [
          {
            ...zenProvider,
            models: [{ ...solModel, enabled: false }, offModel],
          },
          goProvider,
          unconfigured,
        ],
      }),
    );
    render(<ModelsSection />);

    fireEvent.click(
      within(toggleFor("GPT-6 Sol")).getByRole("button", { name: "Off" }),
    );

    await waitFor(() => {
      expect(mockedEnabled).toHaveBeenCalledWith("opencode/gpt-6-sol", false);
    });
    await waitFor(() => {
      expect(
        within(toggleFor("GPT-6 Sol"))
          .getByRole("button", { name: "Off" })
          .getAttribute("aria-pressed"),
      ).toBe("true");
    });
  });

  it("turns a switched-off model back on", async () => {
    render(<ModelsSection />);

    fireEvent.click(
      within(toggleFor("Kimi K2")).getByRole("button", { name: "On" }),
    );

    await waitFor(() => {
      expect(mockedEnabled).toHaveBeenCalledWith("opencode/kimi-k2", true);
    });
  });

  it("renders a glyph per modality a model takes beyond text", () => {
    seedLlm(
      llmDoc({
        providers: [
          {
            ...zenProvider,
            models: [
              { ...solModel, modalities: ["text", "image", "pdf"] },
              offModel,
            ],
          },
          {
            ...goProvider,
            models: [
              model({
                ref: "opencode-go/mimo",
                label: "MiMo",
                modalities: ["text", "image", "video", "audio"],
              }),
            ],
          },
          unconfigured,
        ],
      }),
    );
    render(<ModelsSection />);

    expect(modalityIcon("GPT-6 Sol", "image")).toBeTruthy();
    expect(modalityIcon("GPT-6 Sol", "pdf")).toBeTruthy();
    // Not declared → no glyph, no placeholder.
    expect(modalityIcon("GPT-6 Sol", "video")).toBeNull();
    expect(modalityIcon("GPT-6 Sol", "audio")).toBeNull();
    // `text` is universal, so it never spends a glyph.
    expect(modalityIcon("GPT-6 Sol", "text")).toBeNull();

    for (const modality of ["image", "video", "audio"]) {
      expect(modalityIcon("MiMo", modality)).toBeTruthy();
    }
    expect(modalityIcon("MiMo", "pdf")).toBeNull();
  });

  it("keeps a pill on every row when two providers share a label", () => {
    // "GPT-6 Luna" ships on both OpenCode hosts. A label-keyed layoutId makes
    // one row's pill the lead for both, so the duplicate renders none.
    seedLlm(
      llmDoc({
        providers: [
          {
            ...zenProvider,
            models: [
              model({
                ref: "opencode/gpt-6-luna",
                id: "gpt-6-luna",
                label: "GPT-6 Luna",
              }),
            ],
          },
          {
            ...goProvider,
            models: [
              model({
                ref: "opencode-go/gpt-6-luna",
                id: "gpt-6-luna",
                label: "GPT-6 Luna",
                enabled: false,
              }),
            ],
          },
          unconfigured,
        ],
      }),
    );
    render(<ModelsSection />);

    const toggles = screen.getAllByRole("group", { name: "Enable GPT-6 Luna" });
    expect(toggles).toHaveLength(2);
    // Each row keeps its own selection: on for Zen, off for Go.
    expect(
      within(toggles[0])
        .getByRole("button", { name: "On" })
        .getAttribute("aria-pressed"),
    ).toBe("true");
    expect(
      within(toggles[1])
        .getByRole("button", { name: "On" })
        .getAttribute("aria-pressed"),
    ).toBe("false");
    expect(
      within(toggles[1])
        .getByRole("button", { name: "Off" })
        .getAttribute("aria-pressed"),
    ).toBe("true");
    // Both rows paint their own pill, and each pill carries its own layoutId:
    // two rows sharing one id is the reported bug (the follower renders none).
    const ids = toggles.map((toggle) => {
      const pill = toggle.querySelector("[data-layout-id]");
      expect(pill).toBeTruthy();
      return pill?.getAttribute("data-layout-id");
    });
    expect(ids).toEqual([
      "model-toggle-pill-opencode/gpt-6-luna",
      "model-toggle-pill-opencode-go/gpt-6-luna",
    ]);
  });

  it("routes a toggle on a duplicate label to that row's own ref", async () => {
    seedLlm(
      llmDoc({
        providers: [
          {
            ...zenProvider,
            models: [
              model({
                ref: "opencode/gpt-6-luna",
                id: "gpt-6-luna",
                label: "GPT-6 Luna",
              }),
            ],
          },
          {
            ...goProvider,
            models: [
              model({
                ref: "opencode-go/gpt-6-luna",
                id: "gpt-6-luna",
                label: "GPT-6 Luna",
                enabled: false,
              }),
            ],
          },
          unconfigured,
        ],
      }),
    );
    render(<ModelsSection />);

    const toggles = screen.getAllByRole("group", { name: "Enable GPT-6 Luna" });
    fireEvent.click(within(toggles[1]).getByRole("button", { name: "On" }));

    await waitFor(() => {
      expect(mockedEnabled).toHaveBeenCalledWith("opencode-go/gpt-6-luna", true);
    });
  });
});

import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type {
  CatalogModelDto,
  CatalogProviderDto,
  LlmSettings,
} from "../../../api/settings";
import { useSettingsStore } from "../../../stores/settingsStore";
import { ProvidersSection } from "./ProvidersSection";

vi.mock("../../../api/settings", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../../api/settings")>();
  return {
    ...actual,
    getLlmSettings: vi.fn(),
    putProviderKey: vi.fn(),
  };
});

import { getLlmSettings, putProviderKey } from "../../../api/settings";

const mockedLlm = vi.mocked(getLlmSettings);
const mockedPut = vi.mocked(putProviderKey);

function catalogModel(patch: Partial<CatalogModelDto> = {}): CatalogModelDto {
  return {
    ref: "opencode-go/deepseek-v4-flash",
    id: "deepseek-v4-flash",
    label: "DeepSeek V4 Flash",
    provider_id: "opencode-go",
    provider_name: "OpenCode Go",
    context_window: 200_000,
    context_window_max: 400_000,
    modalities: ["text", "image"],
    tool_call: true,
    json_output: false,
    enabled: true,
    ...patch,
  };
}

const commandCodeModel = catalogModel({
  ref: "commandcode/deepseek/deepseek-v4-flash",
  id: "deepseek/deepseek-v4-flash",
  provider_id: "commandcode",
  provider_name: "Command Code Goat",
});

function provider(patch: Partial<CatalogProviderDto> = {}): CatalogProviderDto {
  return {
    id: "opencode-go",
    name: "OpenCode Go",
    visible: true,
    configured: false,
    masked_api_key: null,
    endpoint: "https://opencode.example/v1",
    endpoint_type: "responses",
    models: [catalogModel()],
    ...patch,
  };
}

const commandCodeProvider = provider({
  id: "commandcode",
  name: "Command Code Goat",
  configured: true,
  masked_api_key: "sk-***cc",
  endpoint_type: "chat_completions",
  models: [commandCodeModel],
});

const arkProvider = provider({ id: "ark-coding", name: "Ark Coding", models: [] });
const hiddenProvider = provider({
  id: "hidden-one",
  name: "Hidden One",
  visible: false,
  models: [],
});

const baseProviders: CatalogProviderDto[] = [
  provider(),
  commandCodeProvider,
  arkProvider,
  hiddenProvider,
];

/** The wireframe card that owns a provider's key field — its data-configured
 *  attribute is the page's only status readout (no badge). */
function cardFor(providerName: string): HTMLElement {
  const card = screen
    .getByLabelText(`API key for ${providerName}`)
    .closest(".settings-provider-card");
  if (!card) throw new Error(`no provider card for ${providerName}`);
  return card as HTMLElement;
}

function llmDoc(patch: Partial<LlmSettings> = {}): LlmSettings {
  return {
    catalog_path: "C:\\Users\\x\\provider-catalog.toml",
    revision: 3,
    providers: baseProviders,
    active_models: [commandCodeModel],
    ...patch,
  };
}

describe("ProvidersSection", () => {
  beforeEach(() => {
    mockedLlm.mockReset().mockResolvedValue(llmDoc());
    mockedPut.mockReset().mockResolvedValue({ revision: 4, docs: ["llm"] });
    useSettingsStore.setState({
      open: true,
      section: "connection",
      revision: 3,
      llm: llmDoc(),
      persistByDoc: {},
      docClock: {},
      loadError: null,
    });
  });

  afterEach(() => {
    cleanup();
    vi.useRealTimers();
  });

  it("lists every visible provider up front, with no add control", () => {
    render(<ProvidersSection />);

    // The catalog is the only source of providers, so there is nothing to add.
    expect(screen.queryByRole("button", { name: "Add provider" })).toBeNull();

    expect(screen.getByLabelText("API key for OpenCode Go")).toBeTruthy();
    expect(screen.getByLabelText("API key for Command Code Goat")).toBeTruthy();
    expect(screen.getByLabelText("API key for Ark Coding")).toBeTruthy();
    // visible=false stays out of the page entirely.
    expect(screen.queryByLabelText("API key for Hidden One")).toBeNull();

    // Status is carried by the card itself, not by a badge.
    expect(cardFor("Ark Coding").getAttribute("data-configured")).toBe("false");

    // An empty field says the key is empty, and the catalog file — the
    // backend's own config — is never surfaced.
    const arkField = screen.getByLabelText("API key for Ark Coding") as HTMLInputElement;
    expect(arkField.placeholder).toBe("Empty key");
    expect(screen.queryByText(/provider-catalog\.toml/)).toBeNull();
  });

  it("shows stored credentials as a masked echo, never as a value", () => {
    render(<ProvidersSection />);

    const field = screen.getByLabelText(
      "API key for Command Code Goat",
    ) as HTMLInputElement;
    expect(field.value).toBe("");
    expect(field.placeholder).toBe("Current: sk-***cc");
    expect(cardFor("Command Code Goat").getAttribute("data-configured")).toBe(
      "true",
    );
  });

  it("offers no way to remove a credential: a key is only ever overwritten", () => {
    render(<ProvidersSection />);
    expect(screen.queryByRole("button", { name: /Remove/i })).toBeNull();
  });

  it("leaves models to the Models page", () => {
    render(<ProvidersSection />);
    expect(screen.queryByText("commandcode/deepseek/deepseek-v4-flash")).toBeNull();
  });

  it("never PUTs while a key field is empty", async () => {
    vi.useFakeTimers();
    render(<ProvidersSection />);

    await vi.advanceTimersByTimeAsync(1200);
    expect(mockedPut).not.toHaveBeenCalled();
    expect(screen.queryByText("Fix fields to save")).toBeNull();
  });

  it("saves a typed key through the provider key API only", async () => {
    vi.useFakeTimers();
    mockedLlm.mockResolvedValue(
      llmDoc({
        revision: 4,
        providers: baseProviders.map((p) =>
          p.id === "opencode-go"
            ? { ...p, configured: true, masked_api_key: "sk-***test" }
            : p,
        ),
      }),
    );
    render(<ProvidersSection />);

    fireEvent.change(screen.getByLabelText("API key for OpenCode Go"), {
      target: { value: "sk-test" },
    });
    // Debounced autosave (400ms) → PUT → refetch llm.
    await vi.advanceTimersByTimeAsync(400);
    await vi.advanceTimersByTimeAsync(0);
    await vi.advanceTimersByTimeAsync(0);

    expect(mockedPut).toHaveBeenCalledTimes(1);
    expect(mockedPut).toHaveBeenCalledWith("opencode-go", "sk-test");
    // The llm document is authoritative: the card turns configured from it.
    expect(useSettingsStore.getState().llm?.providers[0]?.configured).toBe(true);
    expect(cardFor("OpenCode Go").getAttribute("data-configured")).toBe("true");
  });
});

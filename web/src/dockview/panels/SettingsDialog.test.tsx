import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { LlmSettings } from "../../api/settings";
import { EMPTY_SLICE, useTurnStore } from "../../stores/turnStore";
import { useSettingsStore } from "../../stores/settingsStore";
import { SettingsDialog } from "./SettingsDialog";

vi.mock("../../api/workspace", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../api/workspace")>();
  return {
    ...actual,
    getEnginesDetail: vi.fn(() => new Promise(() => {})),
    getEngines: vi.fn(() => new Promise(() => {})),
  };
});

const llmDoc: LlmSettings = {
  catalog_path: "C:\\x\\provider-catalog.toml",
  revision: 1,
  providers: [
    {
      id: "openai",
      name: "OpenAI",
      visible: true,
      configured: false,
      masked_api_key: null,
      endpoint: "https://api.openai.com/v1",
      endpoint_type: "responses",
      models: [],
    },
  ],
  active_models: [],
};

vi.mock("../../api/settings", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../api/settings")>();
  return {
    ...actual,
    getSettingsSummary: vi.fn(),
    getLlmSettings: vi.fn(async () => llmDoc),
    putProviderKey: vi.fn(),
    deleteProviderKey: vi.fn(),
    putModelEnabled: vi.fn(),
    getAgent: vi.fn(async () => ({})),
    loadSettingsAgentIds: vi.fn(async () => ["default"]),
    getMcpServers: vi.fn(async () => ({ global: [], workspace: [] })),
    getAvailableTools: vi.fn(),
    getCustomTools: vi.fn(),
    getLog: vi.fn(),
    getExcludes: vi.fn(),
    getWebSearch: vi.fn(),
  };
});

import { getEnginesDetail } from "../../api/workspace";

describe("SettingsDialog", () => {
  beforeEach(() => {
    useTurnStore.setState({ byId: new Map() });
    useSettingsStore.setState({
      open: true,
      section: "connection",
      revision: 1,
      llm: llmDoc,
      persistByDoc: {},
      loadError: null,
      docClock: { llm: 1 },
    });
  });

  afterEach(() => {
    cleanup();
  });

  it("renders Provider while engines/detail is hung and does not fetch detail", () => {
    render(<SettingsDialog />);
    expect(screen.getByText("Providers")).toBeTruthy();
    expect(screen.getByRole("button", { name: /Provider/ })).toBeTruthy();
    // Two LLM pages: credentials and the models those credentials unlock.
    expect(screen.getByRole("button", { name: /^Models/ })).toBeTruthy();
    expect(getEnginesDetail).not.toHaveBeenCalled();
  });

  it("shows a turn banner when a session is running and keeps navigation clickable", () => {
    render(<SettingsDialog />);
    expect(
      screen.queryByText(/settings saves are disabled/i),
    ).toBeNull();

    act(() => {
      useTurnStore.setState({
        byId: new Map([["sess", { ...EMPTY_SLICE, runState: "running" }]]),
      });
    });

    expect(
      screen.getByText(/settings saves are disabled/i),
    ).toBeTruthy();
    const provider = screen.getByRole("button", { name: /Provider/ });
    expect(provider.hasAttribute("disabled")).toBe(false);
    fireEvent.click(provider);
    expect(provider.hasAttribute("disabled")).toBe(false);
  });
});

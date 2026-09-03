import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { AdapterDescriptor, ModelDefinition, ProviderView } from "../../../api/settings";
import { useSettingsStore } from "../../../stores/settingsStore";
import { ModelsSection } from "./ModelsSection";

const adapters: AdapterDescriptor[] = [
  {
    id: "openai",
    label: "OpenAI",
    provider_fields: [
      { name: "endpoint", label: "Endpoint", type: "string", required: true },
    ],
    model_fields: [],
    default_endpoint: "https://api.openai.com/v1",
  },
];

const readyProvider: ProviderView = {
  id: "prov",
  adapter_id: "openai",
  label: "Prod",
  endpoint: "https://api.openai.com/v1",
  api_key: "sk-test",
  auth: "bearer",
};

const savedModel: ModelDefinition = {
  id: "saved",
  adapter_id: "openai",
  provider_ref: "prov",
  label: "Saved model",
  config: {
    api_model_id: "gpt-4o",
    context_window: 200_000,
    max_tokens: 8192,
    capabilities: ["text"],
  },
};

describe("ModelsSection persist UX", () => {
  const saveModels = vi.fn(async () => undefined);

  beforeEach(() => {
    saveModels.mockClear();
    useSettingsStore.setState({
      adapters,
      providers: { prov: readyProvider },
      models: {},
      agents: {},
      persistByDoc: {},
      saveModels,
    });
  });

  afterEach(() => {
    cleanup();
    vi.useRealTimers();
  });

  it("does not show Fix fields to save or PUT when adding an incomplete model", async () => {
    vi.useFakeTimers();
    render(<ModelsSection />);
    fireEvent.click(screen.getByRole("button", { name: "Add model" }));
    await vi.advanceTimersByTimeAsync(400);
    expect(screen.queryByText("Fix fields to save")).toBeNull();
    expect(saveModels).not.toHaveBeenCalled();
  });

  it("commits once the new model has provider and api model id", async () => {
    vi.useFakeTimers();
    render(<ModelsSection />);
    fireEvent.click(screen.getByRole("button", { name: "Add model" }));
    const apiId = screen
      .getAllByRole("textbox")
      .find((el) => (el as HTMLInputElement).type !== "number" && (el as HTMLInputElement).value === "");
    expect(apiId).toBeTruthy();
    fireEvent.change(apiId!, { target: { value: "gpt-4o" } });
    await vi.advanceTimersByTimeAsync(400);
    expect(saveModels).toHaveBeenCalledTimes(1);
    const payload = saveModels.mock.calls[0][0] as Record<string, ModelDefinition>;
    expect(Object.values(payload)[0]?.config.api_model_id).toBe("gpt-4o");
  });

  it("PUTs remaining models after Remove", async () => {
    vi.useFakeTimers();
    useSettingsStore.setState({ models: { saved: savedModel } });
    render(<ModelsSection />);
    fireEvent.click(screen.getByRole("button", { name: "Remove" }));
    await vi.advanceTimersByTimeAsync(400);
    expect(saveModels).toHaveBeenCalledWith({});
  });
});

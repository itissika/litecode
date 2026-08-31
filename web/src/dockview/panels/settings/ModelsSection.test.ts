import { describe, expect, it } from "vitest";

import type { ModelDefinition } from "../../../api/settings";
import { serializeModels } from "./ModelsSection";

function draftModel(patch: Partial<ModelDefinition> = {}): ModelDefinition {
  return {
    id: "model_1",
    adapter_id: "openai",
    provider_ref: "",
    label: "New model",
    config: {
      api_model_id: "",
      context_window: 200_000,
      max_tokens: 8192,
      thinking_mode: null,
      reasoning_effort: null,
      json_output: false,
      capabilities: ["text"],
    },
    ...patch,
  };
}

describe("serializeModels", () => {
  it("omits a brand-new incomplete model instead of marking the form invalid", () => {
    expect(serializeModels({ model_1: draftModel() }, new Set())).toEqual({ ok: {} });
  });

  it("marks invalid when a previously saved model loses required fields", () => {
    expect(
      serializeModels({ model_1: draftModel() }, new Set(["model_1"])),
    ).toEqual({ skip: "invalid" });
  });

  it("keeps saved models while a blank new card is still empty", () => {
    const saved = draftModel({
      id: "saved",
      provider_ref: "prov",
      config: {
        api_model_id: "gpt-4o",
        context_window: 200_000,
        max_tokens: 8192,
        capabilities: ["text"],
      },
    });
    expect(
      serializeModels({ saved, model_1: draftModel() }, new Set(["saved"])),
    ).toEqual({ ok: { saved } });
  });

  it("serializes a complete new model", () => {
    const model = draftModel({
      provider_ref: "prov",
      config: {
        api_model_id: "gpt-4o",
        context_window: 200_000,
        max_tokens: 8192,
        capabilities: ["text"],
      },
    });
    expect(serializeModels({ model_1: model }, new Set())).toEqual({
      ok: { model_1: model },
    });
  });
});

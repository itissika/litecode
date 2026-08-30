import { describe, expect, it } from "vitest";

import type { AdapterDescriptor } from "../../../api/settings";
import {
  serializeProviderDrafts,
  type ProviderDraft,
} from "./ConnectionSection";

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

function blankNewDraft(patch: Partial<ProviderDraft> = {}): ProviderDraft {
  return {
    id: "provider_1",
    adapter_id: "openai",
    label: "",
    endpoint: "https://api.openai.com/v1",
    api_key: "",
    auth: "bearer",
    masked_key: null,
    ...patch,
  };
}

describe("serializeProviderDrafts", () => {
  it("omits a brand-new empty card instead of marking the form invalid", () => {
    const result = serializeProviderDrafts([blankNewDraft()], adapters);
    expect(result).toEqual({ ok: {} });
  });

  it("keeps saved providers while a blank new card is still empty", () => {
    const saved: ProviderDraft = blankNewDraft({
      id: "saved",
      label: "Prod",
      masked_key: "sk-…abcd",
    });
    const result = serializeProviderDrafts([saved, blankNewDraft()], adapters);
    expect(result).toEqual({
      ok: {
        saved: {
          id: "saved",
          adapter_id: "openai",
          label: "Prod",
          config: {
            endpoint: "https://api.openai.com/v1",
            api_key: "sk-…abcd",
            auth: "bearer",
          },
        },
      },
    });
  });

  it("marks invalid once the new card is started but still missing a key", () => {
    const result = serializeProviderDrafts(
      [blankNewDraft({ label: "My provider" })],
      adapters,
    );
    expect(result).toEqual({ skip: "invalid" });
  });

  it("serializes a complete new provider", () => {
    const result = serializeProviderDrafts(
      [blankNewDraft({ label: "Mine", api_key: "sk-test" })],
      adapters,
    );
    expect(result).toMatchObject({
      ok: {
        provider_1: {
          id: "provider_1",
          label: "Mine",
          config: { api_key: "sk-test" },
        },
      },
    });
  });
});

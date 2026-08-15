import { useEffect, useMemo, useState, type FormEvent, type ReactNode } from "react";
import { Plus } from "@phosphor-icons/react";

import {
  type AdapterDescriptor,
  type ModelDefinition,
  type ProviderView,
} from "../../../api/settings";
import { useSettingsStore } from "../../../stores/settingsStore";
import { Select } from "../../../components/ui/Select";
import { FoldCard } from "../../../components/FoldCard";
import {
  FieldLabel,
  TextInput,
  SettingsPageShell,
  adapterDefaultEndpoint,
} from "./shared";

function readyProviderIds(
  providers: Record<string, ProviderView> | null,
  adapters: AdapterDescriptor[],
  adapterId: string,
): string[] {
  const defaultEndpoint = adapterDefaultEndpoint(adapters, adapterId);
  return Object.values(providers ?? {})
    .filter((p) => {
      if (p.adapter_id !== adapterId) return false;
      const endpoint = (p.endpoint?.trim() ?? "") || defaultEndpoint;
      return endpoint.length > 0 && (p.api_key?.trim() ?? "").length > 0;
    })
    .map((p) => p.id)
    .sort();
}

// Closed adapters expose `api_model_id` as a fixed catalog (enum with options),
// not free text. Deriving this from the fetched `model_fields` schema keeps the
// front-end in lock-step with the back-end adapter registry — no duplicated,
// hand-synced adapter-id list that can drift.
function isClosedAdapter(
  adapters: AdapterDescriptor[],
  adapterId: string,
): boolean {
  const apiField = adapters
    .find((a) => a.id === adapterId)
    ?.model_fields.find((f) => f.name === "api_model_id");
  return (apiField?.options?.length ?? 0) > 0;
}

function closedApiModelOptions(
  adapters: AdapterDescriptor[],
  adapterId: string,
): string[] {
  const field = adapters
    .find((a) => a.id === adapterId)
    ?.model_fields.find((f) => f.name === "api_model_id");
  return field?.options ?? [];
}

export function ModelsSection() {
  const adapters = useSettingsStore((s) => s.adapters);
  const providers = useSettingsStore((s) => s.providers);
  const models = useSettingsStore((s) => s.models);
  const agents = useSettingsStore((s) => s.agents);
  const saving = useSettingsStore((s) => s.saving);
  const saveBlocked = useSettingsStore((s) => s.isSaveBlocked());
  const saveModels = useSettingsStore((s) => s.saveModels);
  const [draft, setDraft] = useState<Record<string, ModelDefinition>>({});

  useEffect(() => {
    setDraft(models ?? {});
  }, [models]);

  const referencedModelRefs = useMemo(() => {
    const refs = new Set<string>();
    for (const profile of Object.values(agents)) {
      if (profile.model_ref) refs.add(profile.model_ref);
    }
    return refs;
  }, [agents]);

  const entries = useMemo(
    () => Object.values(draft).sort((a, b) => a.id.localeCompare(b.id)),
    [draft],
  );

  // All ready providers across every adapter. The model's adapter_id is derived
  // from the selected provider, so the provider dropdown is not filtered by adapter.
  const allReadyProviderIds = useMemo(
    () =>
      Array.from(
        new Set(adapters.flatMap((a) => readyProviderIds(providers, adapters, a.id))),
      ).sort(),
    [adapters, providers],
  );

  const updateModel = (id: string, patch: Partial<ModelDefinition>) => {
    setDraft((prev) => ({
      ...prev,
      [id]: { ...prev[id], ...patch, id },
    }));
  };

  const updateConfig = (
    id: string,
    patch: Partial<ModelDefinition["config"]>,
  ) => {
    setDraft((prev) => {
      const cur = prev[id];
      if (!cur) return prev;
      return {
        ...prev,
        [id]: { ...cur, config: { ...cur.config, ...patch } },
      };
    });
  };

  const addModel = () => {
    const firstReady = allReadyProviderIds[0];
    const adapter_id = firstReady ? (providers ?? {})[firstReady]?.adapter_id ?? "" : "";
    const closed = isClosedAdapter(adapters, adapter_id);
    const catalog = closedApiModelOptions(adapters, adapter_id);
    const id = `model_${Date.now()}`;
    setDraft((prev) => ({
      ...prev,
      [id]: {
        id,
        adapter_id,
        provider_ref: firstReady ?? "",
        label: "New model",
        config: {
          api_model_id: closed ? (catalog[0] ?? "") : "",
          context_window: closed ? 0 : 200_000,
          max_tokens: closed ? 0 : 8192,
          thinking_mode: null,
          reasoning_effort: null,
          json_output: false,
          capabilities: ["text"],
        },
      },
    }));
  };

  const removeModel = (id: string) => {
    setDraft((prev) => {
      const next = { ...prev };
      delete next[id];
      return next;
    });
  };

  const onSubmit = (e: FormEvent) => {
    e.preventDefault();
    void saveModels(draft);
  };

  return (
    <SettingsPageShell
      title="Models"
      onSubmit={onSubmit}
      actions={
        <button
          type="button"
          onClick={addModel}
          disabled={saveBlocked || adapters.length === 0}
          className="btn btn-icon"
          aria-label="Add model"
          title="Add model"
        >
          <Plus size={16} />
        </button>
      }
      save={{ disabled: saveBlocked, saving }}
    >
      <div className="space-y-2">
        {entries.map((model) => {
          const selectedAdapter = (providers ?? {})[model.provider_ref]?.adapter_id ?? "";
          const closed = isClosedAdapter(adapters, selectedAdapter);
          const apiCatalog = closedApiModelOptions(adapters, selectedAdapter);
          const providerOptions = [
            { value: "", label: "— select —" as ReactNode },
            ...allReadyProviderIds.map((id) => ({ value: id, label: id as ReactNode })),
          ];
          const apiModelOptions = [
            { value: "", label: "— select —" as ReactNode },
            ...apiCatalog.map((id) => ({ value: id, label: id as ReactNode })),
          ];
          return (
            <FoldCard
              key={model.id}
              defaultOpen={false}
              label={
                <span className="flex flex-1 items-center justify-between gap-2">
                  <span className="font-mono text-sm text-(--_dk-text-secondary)">
                    {model.label.trim() || model.id || "New model"}
                  </span>
                  <button
                    type="button"
                    onClick={(e) => {
                      e.stopPropagation();
                      removeModel(model.id);
                    }}
                    onKeyDown={(e) => e.stopPropagation()}
                    disabled={saveBlocked || referencedModelRefs.has(model.id)}
                    title={
                      referencedModelRefs.has(model.id)
                        ? "An agent still references this model — change its model_ref first"
                        : undefined
                    }
                    className="btn-danger btn-sm"
                  >
                    Remove
                  </button>
                </span>
              }
              className="settings-foldcard"
            >
              <div className="space-y-2">
                {/* Row 1: provider + api model id */}
                <div className="grid grid-cols-2 gap-2">
                  <div>
                    <FieldLabel required>Provider</FieldLabel>
                    <Select
                      value={model.provider_ref}
                      onChange={(v) => {
                        const prov = (providers ?? {})[v];
                        const nextAdapter = prov?.adapter_id ?? "";
                        const nextClosed = isClosedAdapter(adapters, nextAdapter);
                        const catalog = closedApiModelOptions(adapters, nextAdapter);
                        const currentApi = model.config?.api_model_id ?? "";
                        const nextApi = nextClosed
                          ? catalog.includes(currentApi)
                            ? currentApi
                            : (catalog[0] ?? "")
                          : currentApi;
                        setDraft((prev) => {
                          const cur = prev[model.id];
                          if (!cur) return prev;
                          return {
                            ...prev,
                            [model.id]: {
                              ...cur,
                              adapter_id: nextAdapter,
                              provider_ref: v,
                              config: {
                                ...cur.config,
                                api_model_id: nextApi,
                                context_window: nextClosed
                                  ? 0
                                  : cur.config?.context_window || 200_000,
                                max_tokens: nextClosed
                                  ? cur.config?.max_tokens || 0
                                  : cur.config?.max_tokens || 8192,
                              },
                            },
                          };
                        });
                      }}
                      options={providerOptions}
                      disabled={saveBlocked || allReadyProviderIds.length === 0}
                      className="w-full"
                    />
                  </div>
                  {closed ? (
                    <div>
                      <FieldLabel required>API model id</FieldLabel>
                      <Select
                        value={model.config?.api_model_id ?? ""}
                        onChange={(v) => updateConfig(model.id, { api_model_id: v })}
                        options={apiModelOptions}
                        disabled={saveBlocked || apiCatalog.length === 0}
                        className="w-full"
                      />
                    </div>
                  ) : (
                    <div>
                      <FieldLabel required>API model ID</FieldLabel>
                      <TextInput
                        value={model.config?.api_model_id ?? ""}
                        onChange={(e) =>
                          updateConfig(model.id, { api_model_id: e.target.value })
                        }
                        disabled={saveBlocked}
                      />
                    </div>
                  )}
                </div>
                {/* Rows 2+: label, context window (open only), max tokens — 2 per row */}
                <div className="grid grid-cols-2 gap-2">
                  <div>
                    <FieldLabel>Label</FieldLabel>
                    <TextInput
                      value={model.label}
                      onChange={(e) => updateModel(model.id, { label: e.target.value })}
                      disabled={saveBlocked}
                    />
                  </div>
                  {closed ? null : (
                    <div>
                      <FieldLabel required>Context window</FieldLabel>
                      <TextInput
                        type="number"
                        value={model.config?.context_window ?? 0}
                        onChange={(e) =>
                          updateConfig(model.id, {
                            context_window: Number(e.target.value) || 0,
                          })
                        }
                        disabled={saveBlocked}
                      />
                    </div>
                  )}
                  <div>
                    <FieldLabel required={!closed}>Max tokens</FieldLabel>
                    <TextInput
                      type="number"
                      value={model.config?.max_tokens ?? 0}
                      onChange={(e) =>
                        updateConfig(model.id, {
                          max_tokens: Number(e.target.value) || 0,
                        })
                      }
                      disabled={saveBlocked}
                    />
                  </div>
                </div>
              </div>
            </FoldCard>
          );
        })}
      </div>
    </SettingsPageShell>
  );
}

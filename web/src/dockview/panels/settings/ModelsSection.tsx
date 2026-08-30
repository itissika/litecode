import { useEffect, useMemo, useState, type ReactNode } from "react";
import { Plus } from "@phosphor-icons/react";

import {
  type AdapterDescriptor,
  type ModelDefinition,
  type ProviderView,
  getProviderModels,
} from "../../../api/settings";
import { useSettingsStore } from "../../../stores/settingsStore";
import { Select } from "../../../components/ui/Select";
import {
  Dropdown,
  dropdownItemClass,
  dropdownItemActiveClass,
} from "../../../components/ui/Dropdown";
import { FoldCard } from "../../../components/FoldCard";
import {
  FieldLabel,
  TextInput,
  SettingsPageShell,
  adapterDefaultEndpoint,
  useSettingsSaveBlocked,
} from "./shared";
import {
  shouldHydrateDraftFromStore,
  useSettingsPersist,
  type SerializeResult,
} from "./persist";

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

function hasRemoteModelCatalog(
  adapters: AdapterDescriptor[],
  adapterId: string,
): boolean {
  return adapters.find((a) => a.id === adapterId)?.remote_model_catalog === true;
}

function Chevron({ open }: { open: boolean }) {
  return (
    <svg
      width="10"
      height="10"
      viewBox="0 0 10 10"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
      strokeLinejoin="round"
      className="shrink-0 text-(--_dk-text-disabled) transition-transform duration-150"
      style={{ transform: open ? "rotate(180deg)" : "rotate(0deg)" }}
    >
      <path d="M3 1.5l4 3.5-4 3.5" />
    </svg>
  );
}

function RemoteCatalogModelId({
  providerId,
  value,
  disabled,
  onChange,
}: {
  providerId: string;
  value: string;
  disabled: boolean;
  onChange: (value: string) => void;
}) {
  const [ids, setIds] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [query, setQuery] = useState("");

  // Reset catalog state when the provider changes so a stale directory from a
  // previous provider is never shown.
  useEffect(() => {
    setIds([]);
    setError(null);
    setQuery("");
  }, [providerId]);

  const load = async () => {
    if (!providerId) {
      setError("Select a provider first");
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const next = await getProviderModels(providerId);
      setIds(next);
    } catch (err) {
      setError(err instanceof Error ? err.message : "catalog refresh failed");
    } finally {
      setBusy(false);
    }
  };

  // Auto-fetch on first open (and re-open after an error); once loaded the
  // cached list shows instantly on later opens.
  const ensureLoaded = () => {
    if (ids.length === 0 && !busy) void load();
  };

  const catalog = value && !ids.includes(value) ? [value, ...ids] : ids;
  const q = query.trim().toLowerCase();
  const filtered = q ? catalog.filter((id) => id.toLowerCase().includes(q)) : catalog;
  const selected = catalog.includes(value);

  return (
    <div>
      <FieldLabel required>API model id</FieldLabel>
      <Dropdown
        variant="select"
        closeOnSelect={false}
        className="w-full"
        trigger={({ open, toggle }) => (
          <button
            type="button"
            disabled={disabled}
            onClick={() => {
              if (!open) ensureLoaded();
              toggle();
            }}
            className="flex w-full items-center justify-between gap-1 border-0 border-b border-(--_dk-line) bg-transparent px-1 py-[0.375rem] text-left text-[0.875rem] text-(--_dk-text-muted) hover:brightness-110 focus-visible:border-(--_dk-line-visible) disabled:opacity-40"
          >
            <span className={selected ? "text-(--_dk-text-primary)" : "text-(--_dk-text-disabled)"}>
              {selected ? value : "— select —"}
            </span>
            <Chevron open={open} />
          </button>
        )}
      >
        {({ close }) => (
          <div className="flex flex-col">
            <div className="sticky top-0 z-10 bg-(--_dk-overlay)">
              <input
                type="text"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder="Filter…"
                autoFocus
                className="w-full rounded-none border-0 bg-transparent px-3 py-1.5 text-[11px] text-(--_dk-text-primary) outline-none focus:shadow-none focus:outline-none placeholder:text-(--_dk-text-disabled)"
              />
              <div className="mx-3 border-t border-(--_dk-line)" />
            </div>
            {busy ? (
              <div className="px-3 py-2 text-[11px] text-(--_dk-text-muted)">Loading…</div>
            ) : error ? (
              <div className="px-3 py-2 text-[11px] text-(--_dk-danger)">
                {error} — reopen to retry
              </div>
            ) : filtered.length === 0 ? (
              <div className="px-3 py-2 text-[11px] text-(--_dk-text-disabled)">
                {q ? "No match" : "No models"}
              </div>
            ) : (
              filtered.map((id) => (
                <button
                  key={id}
                  type="button"
                  className={`${dropdownItemClass} ${id === value ? dropdownItemActiveClass : ""}`}
                  onClick={() => {
                    onChange(id);
                    close();
                  }}
                >
                  {id}
                </button>
              ))
            )}
          </div>
        )}
      </Dropdown>
    </div>
  );
}

export function serializeModels(
  draft: Record<string, ModelDefinition>,
  savedIds: Set<string> = new Set(),
): SerializeResult<Record<string, ModelDefinition>> {
  const complete: Record<string, ModelDefinition> = {};
  for (const model of Object.values(draft)) {
    const incomplete =
      !model.provider_ref.trim() || !(model.config?.api_model_id ?? "").trim();
    if (!incomplete) {
      complete[model.id] = model;
      continue;
    }
    // Unsaved incomplete row: omit so Add does not flash "Fix fields to save".
    if (!savedIds.has(model.id)) continue;
    return { skip: "invalid" };
  }
  return { ok: complete };
}

export function ModelsSection() {
  const adapters = useSettingsStore((s) => s.adapters);
  const providers = useSettingsStore((s) => s.providers);
  const models = useSettingsStore((s) => s.models);
  const agents = useSettingsStore((s) => s.agents);
  const saveBlocked = useSettingsSaveBlocked();
  const persistStatus = useSettingsStore((s) => s.persistStatus);
  const setPersistStatus = useSettingsStore((s) => s.setPersistStatus);
  const saveModels = useSettingsStore((s) => s.saveModels);
  const [draft, setDraft] = useState<Record<string, ModelDefinition>>(
    () => useSettingsStore.getState().models ?? {},
  );

  useEffect(() => {
    if (!shouldHydrateDraftFromStore(persistStatus)) return;
    setDraft(models ?? {});
  }, [models, persistStatus]);

  useSettingsPersist(draft, {
    debounceMs: 400,
    setStatus: setPersistStatus,
    serialize: (d) =>
      serializeModels(d, new Set(Object.keys(useSettingsStore.getState().models ?? {}))),
    commit: (p) => saveModels(p),
    revert: () => setDraft(useSettingsStore.getState().models ?? {}),
  });

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

  return (
    <SettingsPageShell
      title="Models"
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
    >
      <div className="space-y-2">
        {entries.map((model) => {
          const selectedAdapter = (providers ?? {})[model.provider_ref]?.adapter_id ?? "";
          const closed = isClosedAdapter(adapters, selectedAdapter);
          const apiCatalog = closedApiModelOptions(adapters, selectedAdapter);
          const providerOptions = [
            { value: "", label: "— select —" as ReactNode },
            ...allReadyProviderIds.map((id) => ({
              value: id,
              // Show the provider's human-readable label (fall back to the id
              // when the label is empty) — raw ids are hard to read/compare.
              label: (((providers ?? {})[id]?.label?.trim() || id) as ReactNode),
            })),
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
                  ) : hasRemoteModelCatalog(adapters, selectedAdapter) ? (
                    <RemoteCatalogModelId
                      providerId={model.provider_ref}
                      value={model.config?.api_model_id ?? ""}
                      disabled={saveBlocked}
                      onChange={(v) => updateConfig(model.id, { api_model_id: v })}
                    />
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

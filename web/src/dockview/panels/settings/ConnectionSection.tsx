import { useEffect, useMemo, useState, type ReactNode } from "react";
import { Plus } from "@phosphor-icons/react";

import {
  type AdapterDescriptor,
  type ProviderDefinition,
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
import {
  shouldHydrateDraftFromStore,
  useSettingsPersist,
  type SerializeResult,
} from "./persist";

type ProviderDraft = {
  id: string;
  adapter_id: string;
  label: string;
  endpoint: string;
  api_key: string;
  auth: "bearer" | "api_key";
  masked_key: string | null;
};

function endpointFieldRequired(
  adapters: AdapterDescriptor[],
  adapterId: string,
): boolean {
  const field = adapters
    .find((a) => a.id === adapterId)
    ?.provider_fields.find((f) => f.name === "endpoint");
  return field?.required ?? true;
}

function viewToDraft(view: ProviderView, adapters: AdapterDescriptor[]): ProviderDraft {
  const stored = view.endpoint ?? "";
  return {
    id: view.id,
    adapter_id: view.adapter_id,
    label: view.label,
    endpoint: stored.trim() || adapterDefaultEndpoint(adapters, view.adapter_id),
    api_key: "",
    auth: view.auth === "api_key" ? "api_key" : "bearer",
    masked_key: view.api_key,
  };
}

function serializeProviderDrafts(
  drafts: ProviderDraft[],
  adapters: AdapterDescriptor[],
): SerializeResult<Record<string, ProviderDefinition>> {
  for (const d of drafts) {
    if (!d.id.trim() || !d.adapter_id) return { skip: "invalid" };
    if (endpointFieldRequired(adapters, d.adapter_id) && !d.endpoint.trim()) {
      return { skip: "invalid" };
    }
    if (!(d.api_key.trim() || d.masked_key)) return { skip: "invalid" };
  }
  return { ok: draftsToProviders(drafts) };
}

function snapshotProviderDrafts(): ProviderDraft[] {
  const { providers, adapters } = useSettingsStore.getState();
  return Object.values(providers ?? {}).map((view) => viewToDraft(view, adapters));
}

function draftsToProviders(drafts: ProviderDraft[]): Record<string, ProviderDefinition> {
  const out: Record<string, ProviderDefinition> = {};
  for (const d of drafts) {
    const id = d.id.trim();
    if (!id) continue;
    out[id] = {
      id,
      adapter_id: d.adapter_id,
      label: d.label,
      config: {
        endpoint: d.endpoint.trim(),
        api_key: d.api_key.trim() || d.masked_key || "",
        auth: d.auth,
      },
    };
  }
  return out;
}

export function ConnectionSection() {
  const adapters = useSettingsStore((s) => s.adapters);
  const providers = useSettingsStore((s) => s.providers);
  const saveBlocked = useSettingsStore((s) => s.isSaveBlocked());
  const persistStatus = useSettingsStore((s) => s.persistStatus);
  const setPersistStatus = useSettingsStore((s) => s.setPersistStatus);
  const saveProviders = useSettingsStore((s) => s.saveProviders);
  const [drafts, setDrafts] = useState<ProviderDraft[]>([]);
  const [justAddedProviderId, setJustAddedProviderId] = useState<string | null>(null);

  useEffect(() => {
    if (!shouldHydrateDraftFromStore(persistStatus)) return;
    setDrafts(Object.values(providers ?? {}).map((view) => viewToDraft(view, adapters)));
  }, [providers, adapters, persistStatus]);

  useSettingsPersist(drafts, {
    debounceMs: 400,
    setStatus: setPersistStatus,
    serialize: (d) => serializeProviderDrafts(d, adapters),
    commit: (p) => saveProviders(p),
    revert: () => setDrafts(snapshotProviderDrafts()),
  });

  const adapterOptions = useMemo(
    () => adapters.map((a) => ({ value: a.id, label: a.label as ReactNode })),
    [adapters],
  );

  const updateDraft = (index: number, patch: Partial<ProviderDraft>) => {
    setDrafts((prev) => prev.map((row, i) => (i === index ? { ...row, ...patch } : row)));
  };

  const addProvider = () => {
    const adapter_id = adapters[0]?.id ?? "";
    const id = `provider_${Date.now()}`;
    setDrafts((prev) => [
      ...prev,
      {
        id,
        adapter_id,
        label: "",
        endpoint: adapterDefaultEndpoint(adapters, adapter_id),
        api_key: "",
        auth: "bearer",
        masked_key: null,
      },
    ]);
    setJustAddedProviderId(id);
  };

  const removeProvider = (index: number) => {
    setDrafts((prev) => prev.filter((_, i) => i !== index));
  };

  return (
    <SettingsPageShell
      title="Providers"
      actions={
        <button
          type="button"
          onClick={addProvider}
          disabled={saveBlocked || adapters.length === 0}
          className="btn btn-icon"
          aria-label="Add provider"
          title="Add provider"
        >
          <Plus size={16} />
        </button>
      }
    >
        <div className="settings-content-indent space-y-2">
          <div className="space-y-2">
          {drafts.map((row, index) => (
            <FoldCard
              key={index}
              defaultOpen={row.id === justAddedProviderId}
              label={
                <span className="flex flex-1 items-center justify-between gap-2">
                  <span className="font-mono text-sm text-(--_dk-text-secondary)">
                    {row.label.trim() || row.id || "New provider"}
                  </span>
                  <button
                    type="button"
                    onClick={(e) => {
                      e.stopPropagation();
                      removeProvider(index);
                    }}
                    onKeyDown={(e) => e.stopPropagation()}
                    disabled={saveBlocked}
                    className="btn-danger btn-sm"
                  >
                    Remove
                  </button>
                </span>
              }
              className="settings-foldcard"
            >
              <div className="space-y-2">
                <div>
                  <FieldLabel>Label</FieldLabel>
                  <TextInput
                    value={row.label}
                    onChange={(e) => updateDraft(index, { label: e.target.value })}
                    disabled={saveBlocked}
                    placeholder={row.id || "Provider name"}
                  />
                </div>
                <div className="grid grid-cols-2 gap-2">
                  <div>
                    <FieldLabel required>Adapter</FieldLabel>
                    <Select
                      value={row.adapter_id}
                      onChange={(v) => {
                        const prevDefault = adapterDefaultEndpoint(adapters, row.adapter_id);
                        const nextDefault = adapterDefaultEndpoint(adapters, v);
                        const current = row.endpoint.trim();
                        const endpoint =
                          !current || current === prevDefault ? nextDefault : row.endpoint;
                        updateDraft(index, { adapter_id: v, endpoint });
                      }}
                      options={adapterOptions}
                      disabled={saveBlocked || adapterOptions.length === 0}
                      className="w-full"
                    />
                  </div>
                  <div>
                    <FieldLabel required>Auth</FieldLabel>
                    <Select
                      value={row.auth}
                      onChange={(v) =>
                        updateDraft(index, {
                          auth: v === "api_key" ? "api_key" : "bearer",
                        })
                      }
                      options={[
                        { value: "bearer", label: "bearer" },
                        { value: "api_key", label: "api_key" },
                      ]}
                      disabled={saveBlocked}
                      className="w-full"
                    />
                  </div>
                </div>
                <div className="grid grid-cols-2 gap-2">
                  <div>
                    <FieldLabel required={endpointFieldRequired(adapters, row.adapter_id)}>
                      Endpoint
                    </FieldLabel>
                    <TextInput
                      value={row.endpoint}
                      onChange={(e) => updateDraft(index, { endpoint: e.target.value })}
                      placeholder={
                        adapterDefaultEndpoint(adapters, row.adapter_id) ||
                        "https://api.example.com/v1"
                      }
                      disabled={saveBlocked}
                      autoComplete="off"
                    />
                    {row.adapter_id === "opencode" ? (
                      <p className="mt-1 text-xs text-(--_dk-text-tertiary)">
                        Default is Zen. For Go, use{" "}
                        <span className="font-mono">https://opencode.ai/zen/go/v1</span>{" "}
                        (same key if subscribed). Keys at{" "}
                        <a
                          href="https://opencode.ai/auth"
                          target="_blank"
                          rel="noreferrer"
                          className="underline"
                        >
                          opencode.ai/auth
                        </a>
                        .
                      </p>
                    ) : null}
                    {row.adapter_id === "ark_coding" ? (
                      <p className="mt-1 text-xs text-(--_dk-text-tertiary)">
                        Default is the Coding Plan OpenAI path{" "}
                        <span className="font-mono">
                          https://ark.cn-beijing.volces.com/api/coding/v3
                        </span>
                        . Use the Ark console API key. Coding Plan quota is licensed for
                        listed tools; LiteCode is an unofficial client. Do not use the
                        general <span className="font-mono">/api/v3</span> host.
                      </p>
                    ) : null}
                  </div>
                  <div>
                    <FieldLabel required>API key</FieldLabel>
                    <TextInput
                      type="password"
                      value={row.api_key}
                      onChange={(e) => updateDraft(index, { api_key: e.target.value })}
                      placeholder={row.masked_key ? `Current: ${row.masked_key}` : "sk-…"}
                      disabled={saveBlocked}
                      autoComplete="new-password"
                    />
                  </div>
                </div>
              </div>
            </FoldCard>
          ))}
        </div>
        </div>
    </SettingsPageShell>
  );
}

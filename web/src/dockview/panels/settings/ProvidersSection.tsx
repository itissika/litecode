import { useMemo, useState } from "react";

import { type CatalogProviderDto } from "../../../api/settings";
import { useSettingsStore } from "../../../stores/settingsStore";
import { ProviderLogo } from "../../../components/ProviderLogos";
import {
  SettingsPageShell,
  TextInput,
  useSettingsSaveBlocked,
} from "./shared";
import {
  useDocPersist,
  useSettingsPersist,
  type SerializeResult,
} from "./persist";

/**
 * One provider, one credential field. Nothing to add and nothing to remove: a
 * provider exists because the catalog file declares it, and the only thing the
 * Web app owns is its API key.
 *
 * The key is overwrite-only. An empty field is never serialized, so clearing it
 * is not a delete — a stored credential is replaced by typing a new one.
 *
 * The card is a bare wireframe with no background. A card with no stored key
 * is dimmed as a whole, so one glance says which providers still need one
 * (`.settings-provider-card[data-configured]`).
 */
function ProviderCard({
  provider,
  draftKey,
  saveBlocked,
  onKeyChange,
}: {
  provider: CatalogProviderDto;
  draftKey: string;
  saveBlocked: boolean;
  onKeyChange: (key: string) => void;
}) {
  return (
    <div
      className="settings-provider-card"
      data-configured={provider.configured ? "true" : "false"}
    >
      <div className="flex min-w-0 items-center gap-1.5">
        <ProviderLogo providerId={provider.id} />
        <span className="truncate text-dk-lg text-(--_dk-text-primary)">
          {provider.name}
        </span>
      </div>
      <TextInput
        type="password"
        value={draftKey}
        onChange={(e) => onKeyChange(e.target.value)}
        placeholder={
          provider.masked_api_key
            ? `Current: ${provider.masked_api_key}`
            : "Empty key"
        }
        disabled={saveBlocked}
        autoComplete="new-password"
        aria-label={`API key for ${provider.name}`}
      />
    </div>
  );
}

/**
 * Provider page: credentials only, every catalog provider always listed.
 *
 * Models are not shown here — they belong to the Models page, which reads the
 * same document.
 */
export function ProvidersSection() {
  const llm = useSettingsStore((s) => s.llm);
  const saveProviderKey = useSettingsStore((s) => s.saveProviderKey);
  const saveBlocked = useSettingsSaveBlocked();
  const { setPersistStatus } = useDocPersist("llm");

  /** provider id → key typed in the field but not yet stored. */
  const [draftKeys, setDraftKeys] = useState<Record<string, string>>({});

  // `visible: false` is the catalog's way of saying "not offered in the UI", so
  // the page lists exactly the providers the user may hold a key for.
  const providers = useMemo(
    () => (llm?.providers ?? []).filter((provider) => provider.visible),
    [llm],
  );

  // The key field is the only editor: debounced autosave, exactly like the rest
  // of Settings. An empty field is never serialized, so an untouched card can
  // never PUT, and clearing a field is a no-op rather than a delete.
  useSettingsPersist(draftKeys, {
    debounceMs: 400,
    setStatus: setPersistStatus,
    serialize: (keys): SerializeResult<Record<string, string>> => {
      const payload: Record<string, string> = {};
      for (const provider of providers) {
        const key = (keys[provider.id] ?? "").trim();
        if (key) payload[provider.id] = key;
      }
      if (Object.keys(payload).length === 0) return { skip: "unchanged" };
      return { ok: payload };
    },
    commit: async (payload) => {
      for (const [id, key] of Object.entries(payload)) {
        await saveProviderKey(id, key);
        // Stored now: drop the plaintext from local state so the field falls
        // back to the masked echo of the credential the server just accepted.
        setDraftKeys((prev) => {
          if (!(id in prev)) return prev;
          const next = { ...prev };
          delete next[id];
          return next;
        });
      }
    },
    revert: () => {
      // Keep what was typed — the store already toasted the reason.
    },
  });

  return (
    <SettingsPageShell title="Providers">
      <div className="settings-content-indent">
        {providers.length === 0 ? (
          <p className="text-sm text-(--_dk-text-disabled)">
            No providers in the catalog yet.
          </p>
        ) : (
          // Two cards per row: each card is short (name + key), so the grid
          // roughly halves the page's scroll length.
          <div className="grid grid-cols-2 gap-4">
            {providers.map((provider) => (
              <ProviderCard
                key={provider.id}
                provider={provider}
                draftKey={draftKeys[provider.id] ?? ""}
                saveBlocked={saveBlocked}
                onKeyChange={(key) =>
                  setDraftKeys((prev) => ({ ...prev, [provider.id]: key }))
                }
              />
            ))}
          </div>
        )}
      </div>
    </SettingsPageShell>
  );
}

import { useEffect, useMemo, useState } from "react";
import { isCoreCatalogEntry, type ToolCatalogEntry } from "../../../api/settings";
import { useSettingsStore } from "../../../stores/settingsStore";
import { SettingsPageShell } from "./shared";
import { isPersistBusy, useSettingsPersist } from "./persist";

export function ToolCatalogSection() {
  const toolCatalog = useSettingsStore((s) => s.toolCatalog);
  const saveBlocked = useSettingsStore((s) => s.isSaveBlocked());
  const persistStatus = useSettingsStore((s) => s.persistStatus);
  const setPersistStatus = useSettingsStore((s) => s.setPersistStatus);
  const saveToolCatalog = useSettingsStore((s) => s.saveToolCatalog);
  const [draft, setDraft] = useState<Record<string, ToolCatalogEntry>>({});

  useEffect(() => {
    if (isPersistBusy(persistStatus)) return;
    setDraft(toolCatalog ?? {});
  }, [toolCatalog, persistStatus]);

  useSettingsPersist(draft, {
    debounceMs: 0,
    setStatus: setPersistStatus,
    serialize: (d) => ({ ok: d }),
    commit: (p) => saveToolCatalog(p),
    revert: () => setDraft(useSettingsStore.getState().toolCatalog ?? {}),
  });

  const tierRank: Record<string, number> = { core: 0, optional: 1, custom: 2, mcp: 3 };
  const entries = useMemo(
    () =>
      Object.values(draft).sort((a, b) => {
        const r = (tierRank[a.tier] ?? 9) - (tierRank[b.tier] ?? 9);
        return r !== 0 ? r : a.id.localeCompare(b.id);
      }),
    [draft],
  );

  const toggleEnabled = (id: string) => {
    const entry = draft[id];
    if (!entry || isCoreCatalogEntry(entry) || saveBlocked) return;
    setDraft((prev) => ({
      ...prev,
      [id]: { ...entry, catalog_enabled: !entry.catalog_enabled },
    }));
  };

  return (
    <SettingsPageShell title="Tool Catalog">
      <div className="space-y-4">
      <p className="text-xs text-(--_dk-text-muted)">
        Enable optional, custom, and MCP tools here, then bind them on Agents. Catalog alone does
        not give an agent visibility.
      </p>
      <div className="settings-table-wrap">
        <table className="settings-table">
            <thead>
              <tr>
                <th>Tool</th>
                <th>Tier</th>
                <th>Configured</th>
                <th>Init scope</th>
                <th>Catalog enabled</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {entries.map((entry) => {
                const core = isCoreCatalogEntry(entry);
                return (
                  <tr key={entry.id}>
                    <td className="font-mono text-(--_dk-text-secondary)">
                      {entry.id}
                    </td>
                    <td className="text-(--_dk-text-muted)">{entry.tier}</td>
                    <td>
                      <span className={`tag ${entry.readiness === "ready" ? "tag-ok" : "tag-warn"} tag-xs tag-ghost`}>
                        {entry.readiness}
                      </span>
                    </td>
                    <td className="text-(--_dk-text-muted)">{entry.init_scope}</td>
                    <td>
                      {core ? (
                        <span className="text-xs text-(--_dk-text-disabled)">always on</span>
                      ) : (
                        <input
                          type="checkbox"
                          checked={entry.catalog_enabled}
                          onChange={() => toggleEnabled(entry.id)}
                          disabled={saveBlocked}
                          className="accent-(--_dk-accent-hover)"
                        />
                      )}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      </div>
    </SettingsPageShell>
  );
}

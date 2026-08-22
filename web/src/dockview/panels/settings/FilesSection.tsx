import { useEffect, useState } from "react";

import { useSettingsStore } from "../../../stores/settingsStore";
import { useTreeStore } from "../../../stores/treeStore";
import type { WorkspaceExcludesLists } from "../../../api/settings";
import { globsFromText, textFromGlobs } from "../../../utils/excludeGlobs";
import {
  FieldLabel,
  TextArea,
  SectionHeader,
  SettingsPageShell,
} from "./shared";
import { isPersistBusy, useSettingsPersist } from "./persist";

type FilesDraft = {
  filesText: string;
  searchText: string;
  watcherText: string;
  gitIgnore: boolean;
};

export function FilesSection() {
  const excludes = useSettingsStore((s) => s.excludes);
  const saveBlocked = useSettingsStore((s) => s.isSaveBlocked());
  const persistStatus = useSettingsStore((s) => s.persistStatus);
  const setPersistStatus = useSettingsStore((s) => s.setPersistStatus);
  const saveExcludes = useSettingsStore((s) => s.saveExcludes);
  const [draft, setDraft] = useState<FilesDraft>({
    filesText: "",
    searchText: "",
    watcherText: "",
    gitIgnore: true,
  });

  useEffect(() => {
    if (isPersistBusy(persistStatus) || !excludes) return;
    setDraft({
      filesText: textFromGlobs(excludes.files_exclude),
      searchText: textFromGlobs(excludes.search_exclude),
      watcherText: textFromGlobs(excludes.watcher_exclude),
      gitIgnore: excludes.git_ignore,
    });
  }, [excludes, persistStatus]);

  useSettingsPersist<FilesDraft, WorkspaceExcludesLists>(draft, {
    debounceMs: 400,
    setStatus: setPersistStatus,
    serialize: (d): { ok: WorkspaceExcludesLists } | { skip: "unchanged" } => {
      if (!excludes) return { skip: "unchanged" };
      const payload: WorkspaceExcludesLists = {
        files_exclude: globsFromText(d.filesText),
        search_exclude: globsFromText(d.searchText),
        watcher_exclude: globsFromText(d.watcherText),
        git_ignore: d.gitIgnore,
      };
      const same =
        JSON.stringify(payload.files_exclude) === JSON.stringify(excludes.files_exclude)
        && JSON.stringify(payload.search_exclude) === JSON.stringify(excludes.search_exclude)
        && JSON.stringify(payload.watcher_exclude) === JSON.stringify(excludes.watcher_exclude)
        && payload.git_ignore === excludes.git_ignore;
      if (same) return { skip: "unchanged" };
      return { ok: payload };
    },
    commit: async (p) => {
      await saveExcludes(p);
      await useTreeStore.getState().refreshAll();
    },
    revert: () => {
      const snap = useSettingsStore.getState().excludes;
      if (!snap) return;
      setDraft({
        filesText: textFromGlobs(snap.files_exclude),
        searchText: textFromGlobs(snap.search_exclude),
        watcherText: textFromGlobs(snap.watcher_exclude),
        gitIgnore: snap.git_ignore,
      });
    },
  });

  const restoreDefaults = () => {
    const d = excludes?.defaults;
    if (!d) return;
    setDraft({
      filesText: textFromGlobs(d.files_exclude),
      searchText: textFromGlobs(d.search_exclude),
      watcherText: textFromGlobs(d.watcher_exclude),
      gitIgnore: d.git_ignore,
    });
  };

  return (
    <SettingsPageShell
      title="Files"
      actions={
        <button
          type="button"
          className="text-dk-xs text-(--_dk-text-muted) hover:text-(--_dk-text-primary)"
          disabled={saveBlocked || !excludes}
          onClick={restoreDefaults}
        >
          Restore defaults
        </button>
      }
    >
      <div className="space-y-6">
        <p className="text-dk-sm text-(--_dk-text-secondary)">
          These lists live in this workspace (<code>.litecode/excludes.json</code>).
          New workspaces start with the built-in VS Code defaults; edits apply to the
          tree and search immediately.
        </p>

        <div className="settings-card space-y-3 p-4">
          <SectionHeader title="Files exclude" />
          <FieldLabel>Hidden from the explorer and from search</FieldLabel>
          <TextArea
            rows={6}
            value={draft.filesText}
            onChange={(e) => setDraft((d) => ({ ...d, filesText: e.target.value }))}
            placeholder={"**/.git\n**/.DS_Store"}
            disabled={saveBlocked}
            spellCheck={false}
          />
        </div>

        <div className="settings-card space-y-3 p-4">
          <SectionHeader title="Search exclude" />
          <FieldLabel>Additional hides for search, glob, and index (on top of files exclude)</FieldLabel>
          <TextArea
            rows={6}
            value={draft.searchText}
            onChange={(e) => setDraft((d) => ({ ...d, searchText: e.target.value }))}
            placeholder={"**/node_modules\n**/bower_components"}
            disabled={saveBlocked}
            spellCheck={false}
          />
        </div>

        <div className="settings-card space-y-3 p-4">
          <SectionHeader title=".gitignore" />
          <label className="flex cursor-pointer items-center gap-2 text-sm text-(--_dk-text-primary)">
            <input
              type="checkbox"
              className="accent-(--_dk-accent-hover)"
              checked={draft.gitIgnore}
              disabled={saveBlocked}
              onChange={(e) => setDraft((d) => ({ ...d, gitIgnore: e.target.checked }))}
            />
            Honor .gitignore in the explorer and search
          </label>
        </div>

        <div className="settings-card space-y-3 p-4">
          <SectionHeader title="Watcher exclude" />
          <FieldLabel>Paths skipped by the file watcher</FieldLabel>
          <TextArea
            rows={5}
            value={draft.watcherText}
            onChange={(e) => setDraft((d) => ({ ...d, watcherText: e.target.value }))}
            disabled={saveBlocked}
            spellCheck={false}
          />
        </div>
      </div>
    </SettingsPageShell>
  );
}

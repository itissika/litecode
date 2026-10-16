import { useEffect, useState } from "react";

import { useSettingsStore } from "../../../stores/settingsStore";
import { useTreeStore } from "../../../stores/treeStore";
import type { WorkspaceExcludesLists } from "../../../api/settings";
import { globsFromText, textFromGlobs } from "../../../utils/excludeGlobs";
import { FoldCard } from "../../../components/FoldCard";
import {
  FieldLabel,
  TextArea,
  SectionHeader,
  SettingsPageShell,
  useSettingsSaveBlocked,
} from "./shared";
import {
  shouldHydrateDraftFromStore,
  useDocPersist,
  useSettingsPersist,
} from "./persist";

type FilesDraft = {
  filesText: string;
  searchText: string;
  extensionsText: string;
  watcherText: string;
  gitIgnore: boolean;
  explorerGitIgnore: boolean;
};

function snapshotFilesDraft(): FilesDraft {
  const excludes = useSettingsStore.getState().excludes;
  if (!excludes) {
    return {
      filesText: "",
      searchText: "",
      extensionsText: "",
      watcherText: "",
      gitIgnore: true,
      explorerGitIgnore: false,
    };
  }
  return {
    filesText: textFromGlobs(excludes.files_exclude),
    searchText: textFromGlobs(excludes.search_exclude),
    extensionsText: textFromGlobs(excludes.code_index_extensions),
    watcherText: textFromGlobs(excludes.watcher_exclude),
    gitIgnore: excludes.git_ignore,
    explorerGitIgnore: excludes.explorer_git_ignore,
  };
}

export function FilesSection() {
  const excludes = useSettingsStore((s) => s.excludes);
  const saveBlocked = useSettingsSaveBlocked();
  const { persistStatus, setPersistStatus } = useDocPersist("excludes");
  const saveExcludes = useSettingsStore((s) => s.saveExcludes);
  const [draft, setDraft] = useState<FilesDraft>(snapshotFilesDraft);

  useEffect(() => {
    if (!shouldHydrateDraftFromStore(persistStatus) || !excludes) return;
    setDraft({
      filesText: textFromGlobs(excludes.files_exclude),
      searchText: textFromGlobs(excludes.search_exclude),
      extensionsText: textFromGlobs(excludes.code_index_extensions),
      watcherText: textFromGlobs(excludes.watcher_exclude),
      gitIgnore: excludes.git_ignore,
      explorerGitIgnore: excludes.explorer_git_ignore,
    });
  }, [excludes, persistStatus]);

  useSettingsPersist<FilesDraft, WorkspaceExcludesLists>(draft, {
    debounceMs: 400,
    setStatus: setPersistStatus,
    serialize: (d) => ({
      ok: {
        files_exclude: globsFromText(d.filesText),
        search_exclude: globsFromText(d.searchText),
        watcher_exclude: globsFromText(d.watcherText),
        code_index_extensions: globsFromText(d.extensionsText),
        git_ignore: d.gitIgnore,
        explorer_git_ignore: d.explorerGitIgnore,
      },
    }),
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
        extensionsText: textFromGlobs(snap.code_index_extensions),
        watcherText: textFromGlobs(snap.watcher_exclude),
        gitIgnore: snap.git_ignore,
        explorerGitIgnore: snap.explorer_git_ignore,
      });
    },
  });

  const restoreDefaults = () => {
    const d = excludes?.defaults;
    if (!d) return;
    setDraft({
      filesText: textFromGlobs(d.files_exclude),
      searchText: textFromGlobs(d.search_exclude),
      extensionsText: textFromGlobs(d.code_index_extensions),
      watcherText: textFromGlobs(d.watcher_exclude),
      gitIgnore: d.git_ignore,
      explorerGitIgnore: d.explorer_git_ignore,
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
      <div className="space-y-3">
        <p className="text-dk-sm text-(--_dk-text-secondary)">
          Saved in <code>.litecode/excludes.json</code>; changes apply
          immediately. Semantic search uses the suffix list below.
        </p>

        <div className="settings-card space-y-3 p-4">
          <SectionHeader title=".gitignore" />
          <label className="flex cursor-pointer items-center gap-2 text-sm text-(--_dk-text-primary)">
            <input
              type="checkbox"
              className="accent-(--_dk-accent-hover)"
              checked={draft.gitIgnore}
              disabled={saveBlocked}
              onChange={(e) =>
                setDraft((d) => ({ ...d, gitIgnore: e.target.checked }))
              }
            />
            Honor .gitignore in search and code index
          </label>
          <label className="flex cursor-pointer items-center gap-2 text-sm text-(--_dk-text-primary)">
            <input
              type="checkbox"
              className="accent-(--_dk-accent-hover)"
              checked={draft.explorerGitIgnore}
              disabled={saveBlocked}
              onChange={(e) =>
                setDraft((d) => ({ ...d, explorerGitIgnore: e.target.checked }))
              }
            />
            Honor .gitignore in the explorer
          </label>
        </div>

        <FoldCard
          label={<span className="settings-section-title">Files exclude</span>}
          className="settings-foldcard"
        >
          <FieldLabel>Hidden from the explorer and from search</FieldLabel>
          <TextArea
            rows={6}
            value={draft.filesText}
            onChange={(e) =>
              setDraft((d) => ({ ...d, filesText: e.target.value }))
            }
            placeholder={"**/.git\n**/.DS_Store"}
            disabled={saveBlocked}
            spellCheck={false}
          />
        </FoldCard>

        <FoldCard
          label={<span className="settings-section-title">Search exclude</span>}
          className="settings-foldcard"
        >
          <FieldLabel>
            Extra hides for search and glob.
          </FieldLabel>
          <TextArea
            rows={6}
            value={draft.searchText}
            onChange={(e) =>
              setDraft((d) => ({ ...d, searchText: e.target.value }))
            }
            placeholder={"**/node_modules\n**/bower_components"}
            disabled={saveBlocked}
            spellCheck={false}
          />
        </FoldCard>

        <FoldCard
          label={
            <span className="settings-section-title">Code index extensions</span>
          }
          className="settings-foldcard"
        >
          <FieldLabel>
            File suffixes to index, one per line.
          </FieldLabel>
          <TextArea
            rows={6}
            value={draft.extensionsText}
            onChange={(e) =>
              setDraft((d) => ({ ...d, extensionsText: e.target.value }))
            }
            placeholder={"rs\nts\ntsx"}
            disabled={saveBlocked}
            spellCheck={false}
          />
        </FoldCard>

        <FoldCard
          label={
            <span className="settings-section-title">Watcher exclude</span>
          }
          className="settings-foldcard"
        >
          <FieldLabel>Paths skipped by the file watcher</FieldLabel>
          <TextArea
            rows={5}
            value={draft.watcherText}
            onChange={(e) =>
              setDraft((d) => ({ ...d, watcherText: e.target.value }))
            }
            disabled={saveBlocked}
            spellCheck={false}
          />
        </FoldCard>
      </div>
    </SettingsPageShell>
  );
}

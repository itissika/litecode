import { create } from "zustand";
import type { DockviewApi } from "dockview-react";

import {
  readFile,
  writeFile,
  type WorkspaceChangeKind,
} from "../api/workspace";
import { flushMarkdownEditor } from "../lib/markdownFlush";
import { languageFromPath, fileNameFromPath } from "../utils/language";
import { isWysiwygMarkdownPath, type MdEditorView } from "../utils/wysiwygMarkdown";
import { remapPathPrefix } from "../utils/path";
import { closingFlags } from "../dockview/config/sharedFlags";
import { attachSiblingStores } from "./connectionStore";

export interface EditorTab {
  path: string;
  content: string;
  savedContent: string;
  dirty: boolean;
  language: string;
  loading: boolean;
  error: string | null;
}

/** A file the user has open that was overwritten on disk (by the agent).
 *  Kept for transitional UI; agent-first policy reloads from disk instead of
 *  retaining dirty human edits. */
export interface EditorConflict {
  path: string;
  source: string;
}

/** One editor caret location on the browse jump stack. */
export interface JumpLocation {
  path: string;
  line: number;
  column: number;
}

interface EditorStore {
  tabs: EditorTab[];
  conflicts: Record<string, EditorConflict>;
  activePath: string | null;
  saving: boolean;
  dockviewApi: DockviewApi | null;
  pendingReveal: { path: string; line: number; column?: number } | null;
  /** Per-tab Markdown view. Missing means default (wysiwyg for `.md`). */
  mdViewByPath: Record<string, MdEditorView>;
  jumpBack: JumpLocation[];
  jumpForward: JumpLocation[];

  openFile: (path: string) => Promise<void>;
  /** Open file and reveal a 1-based line (workspace search / go-to). */
  openFileAt: (path: string, line: number, column?: number) => Promise<void>;
  consumePendingReveal: () => { path: string; line: number; column?: number } | null;
  pushJump: (from: JumpLocation) => void;
  goJumpBack: (current?: JumpLocation) => JumpLocation | null;
  goJumpForward: (current?: JumpLocation) => JumpLocation | null;
  closeTab: (path: string) => void;
  setActive: (path: string) => void;
  setContent: (path: string, content: string) => void;
  save: (path?: string) => Promise<void>;
  reloadFromDisk: (path: string) => Promise<void>;
  handleWorkspaceChange: (
    paths: string[],
    kind: WorkspaceChangeKind,
  ) => Promise<void>;
  remapTabs: (from: string, to: string) => void;
  closeDeleted: (path: string) => void;
  clearConflict: (path: string) => void;
  setDockviewApi: (api: DockviewApi | null) => void;
  setMdView: (path: string, view: MdEditorView) => void;
}

function makeTab(path: string, content: string): EditorTab {
  return {
    path,
    content,
    savedContent: content,
    dirty: false,
    language: languageFromPath(path),
    loading: false,
    error: null,
  };
}

export const useEditorStore = create<EditorStore>((set, get) => ({
  tabs: [],
  conflicts: {},
  activePath: null,
  saving: false,
  dockviewApi: null,
  pendingReveal: null,
  mdViewByPath: {},
  jumpBack: [],
  jumpForward: [],

  setDockviewApi: (api) => set({ dockviewApi: api }),

  setMdView: (path, view) => {
    set((s) => ({
      mdViewByPath: { ...s.mdViewByPath, [path]: view },
    }));
  },

  pushJump: (from) => {
    set((s) => ({
      jumpBack: [...s.jumpBack.slice(-99), from],
      jumpForward: [],
    }));
  },

  goJumpBack: (current) => {
    const s = get();
    if (s.jumpBack.length === 0) return null;
    const loc = s.jumpBack[s.jumpBack.length - 1];
    set({
      jumpBack: s.jumpBack.slice(0, -1),
      jumpForward: current ? [...s.jumpForward, current] : s.jumpForward,
    });
    return loc;
  },

  goJumpForward: (current) => {
    const s = get();
    if (s.jumpForward.length === 0) return null;
    const loc = s.jumpForward[s.jumpForward.length - 1];
    set({
      jumpForward: s.jumpForward.slice(0, -1),
      jumpBack: current ? [...s.jumpBack, current] : s.jumpBack,
    });
    return loc;
  },

  openFileAt: async (path, line, column) => {
    set((s) => ({
      pendingReveal: {
        path,
        line: Math.max(1, Math.floor(line)),
        column: column != null ? Math.max(1, Math.floor(column)) : undefined,
      },
      mdViewByPath: isWysiwygMarkdownPath(path)
        ? { ...s.mdViewByPath, [path]: "source" }
        : s.mdViewByPath,
    }));
    await get().openFile(path);
  },

  consumePendingReveal: () => {
    const reveal = get().pendingReveal;
    if (!reveal) return null;
    set({ pendingReveal: null });
    return reveal;
  },

  openFile: async (path) => {
    const { dockviewApi } = get();
    const existing = get().tabs.find((t) => t.path === path);
    if (existing) {
      set({ activePath: path });
      dockviewApi?.getPanel(path)?.api.setActive();
      return;
    }

    // Fallback to pure store mode if dockviewApi is not available
    if (!dockviewApi) {
      const loadingTab: EditorTab = {
        path,
        content: "",
        savedContent: "",
        dirty: false,
        language: languageFromPath(path),
        loading: true,
        error: null,
      };

      set((s) => ({
        tabs: [...s.tabs, loadingTab],
        activePath: path,
      }));

      try {
        const content = await readFile(path);
        set((s) => ({
          tabs: s.tabs.map((t) =>
            t.path === path ? makeTab(path, content) : t,
          ),
        }));
      } catch (e) {
        const msg = e instanceof Error ? e.message : String(e);
        set((s) => ({
          tabs: s.tabs.map((t) =>
            t.path === path ? { ...t, loading: false, error: msg } : t,
          ),
        }));
      }
      return;
    }

    const fileName = fileNameFromPath(path);

    // Check if any grid group exists. If not (first file opened without default editor panel), create one.
    const gridGroups = dockviewApi.groups.filter((g) => g.api.location.type === "grid");
    let panel: ReturnType<typeof dockviewApi.addPanel>;
    if (gridGroups.length === 0) {
      const group = dockviewApi.addGroup();
      panel = dockviewApi.addPanel({
        id: path,
        component: "editor",
        title: fileName,
        tabComponent: "editor",
        params: { filePath: path },
        position: { referenceGroup: group.id },
      });
    } else {
      panel = dockviewApi.addPanel({
        id: path,
        component: "editor",
        title: fileName,
        tabComponent: "editor",
        params: { filePath: path },
        position: { referenceGroup: gridGroups[0].api.id },
      });
    }

    const loadingTab: EditorTab = {
      path,
      content: "",
      savedContent: "",
      dirty: false,
      language: languageFromPath(path),
      loading: true,
      error: null,
    };

    set((s) => ({
      tabs: [...s.tabs, loadingTab],
      activePath: path,
    }));

    try {
      const content = await readFile(path);
      set((s) => ({
        tabs: s.tabs.map((t) =>
          t.path === path ? makeTab(path, content) : t,
        ),
      }));
      panel.api.setTitle(fileName);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      set((s) => ({
        tabs: s.tabs.map((t) =>
          t.path === path ? { ...t, loading: false, error: msg } : t,
        ),
      }));
    }
  },

  closeTab: (path) => {
    const { dockviewApi } = get();

    // Mark that this close is initiated by the store to prevent
    // onDidRemovePanel → closeTab infinite loop.
    closingFlags.closingFromStore = true;
    try {
      dockviewApi?.getPanel(path)?.api.close();
    } finally {
      closingFlags.closingFromStore = false;
    }

    set((s) => {
      const idx = s.tabs.findIndex((t) => t.path === path);
      if (idx < 0) return s;

      const nextTabs = s.tabs.filter((t) => t.path !== path);
      let nextActive = s.activePath;
      if (s.activePath === path) {
        const neighbor = nextTabs[idx] ?? nextTabs[idx - 1];
        nextActive = neighbor?.path ?? null;
      }

      const mdViewByPath = { ...s.mdViewByPath };
      delete mdViewByPath[path];
      return { tabs: nextTabs, activePath: nextActive, mdViewByPath };
    });
  },

  setActive: (path) => {
    const { dockviewApi } = get();
    dockviewApi?.getPanel(path)?.api.setActive();
    set({ activePath: path });
  },

  setContent: (path, content) => {
    set((s) => ({
      tabs: s.tabs.map((t) =>
        t.path === path
          ? {
              ...t,
              content,
              dirty: content !== t.savedContent,
            }
          : t,
      ),
    }));
  },

  save: async (pathArg) => {
    const path = pathArg ?? get().activePath;
    if (!path) return;

    const flushed = flushMarkdownEditor(path);
    if (flushed != null) {
      get().setContent(path, flushed);
    }

    const tab = get().tabs.find((t) => t.path === path);
    if (!tab || tab.loading) return;

    // Freeze the bytes we actually send. Completing a save must never claim
    // later edits (content B) were written when only snapshot A hit disk.
    const sentContent = tab.content;

    set({ saving: true });
    try {
      await writeFile(path, sentContent);
      set((s) => {
        const conflicts = { ...s.conflicts };
        delete conflicts[path];
        return {
          saving: false,
          conflicts,
          tabs: s.tabs.map((t) =>
            t.path === path
              ? {
                  ...t,
                  savedContent: sentContent,
                  dirty: t.content !== sentContent,
                  error: null,
                }
              : t,
          ),
        };
      });
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      set((s) => ({
        saving: false,
        tabs: s.tabs.map((t) =>
          t.path === path ? { ...t, error: msg } : t,
        ),
      }));
    }
  },

  reloadFromDisk: async (path) => {
    try {
      const content = await readFile(path);
      set((s) => ({
        tabs: s.tabs.map((t) =>
          t.path === path ? makeTab(path, content) : t,
        ),
      }));
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      set((s) => ({
        tabs: s.tabs.map((t) =>
          t.path === path ? { ...t, error: msg } : t,
        ),
      }));
    }
  },

  handleWorkspaceChange: async (paths, kind) => {
    if (kind === "renamed" && paths.length >= 2) {
      const [from, to] = paths;
      if (from && to) get().remapTabs(from, to);
      return;
    }

    if (kind === "deleted") {
      for (const p of paths) {
        get().closeDeleted(p);
      }
      return;
    }

    for (const p of paths) {
      const tab = get().tabs.find((t) => t.path === p);
      if (!tab) continue;

      // Agent-first: disk is the authority. Discard unsaved human edits and
      // reload. Conflict cards are intentionally not used.
      set((s) => {
        if (!(p in s.conflicts)) return s;
        const conflicts = { ...s.conflicts };
        delete conflicts[p];
        return { conflicts };
      });
      await get().reloadFromDisk(p);
    }
  },

  remapTabs: (from, to) => {
    const { dockviewApi, tabs } = get();
    const affected = tabs.filter(
      (t) => t.path === from || (from !== "" && t.path.startsWith(`${from}/`)),
    );
    if (affected.length === 0) return;

    set((s) => {
      const nextConflicts: Record<string, EditorConflict> = {};
      for (const [key, value] of Object.entries(s.conflicts)) {
        const nextKey = remapPathPrefix(key, from, to);
        nextConflicts[nextKey] = { ...value, path: nextKey };
      }
      const mdViewByPath: Record<string, MdEditorView> = {};
      for (const [key, value] of Object.entries(s.mdViewByPath)) {
        mdViewByPath[remapPathPrefix(key, from, to)] = value;
      }
      return {
        tabs: s.tabs.map((t) => {
          const path = remapPathPrefix(t.path, from, to);
          if (path === t.path) return t;
          return { ...t, path, language: languageFromPath(path) };
        }),
        activePath: s.activePath
          ? remapPathPrefix(s.activePath, from, to)
          : null,
        conflicts: nextConflicts,
        mdViewByPath,
      };
    });

    if (!dockviewApi) return;

    for (const tab of affected) {
      const oldPath = tab.path;
      const newPath = remapPathPrefix(oldPath, from, to);
      if (oldPath === newPath) continue;
      const panel = dockviewApi.getPanel(oldPath);
      const groupId = panel?.api.group.api.id;
      closingFlags.closingFromStore = true;
      try {
        panel?.api.close();
      } finally {
        closingFlags.closingFromStore = false;
      }
      const gridGroups = dockviewApi.groups.filter(
        (g) => g.api.location.type === "grid",
      );
      const referenceGroup =
        groupId ?? (gridGroups[0] ? gridGroups[0].api.id : undefined);
      if (!referenceGroup) continue;
      dockviewApi.addPanel({
        id: newPath,
        component: "editor",
        title: fileNameFromPath(newPath),
        tabComponent: "editor",
        params: { filePath: newPath },
        position: { referenceGroup },
      });
    }
  },

  closeDeleted: (path) => {
    const victims = get().tabs.filter(
      (t) => t.path === path || (path !== "" && t.path.startsWith(`${path}/`)),
    );
    for (const tab of victims) {
      get().closeTab(tab.path);
    }
  },

  clearConflict: (path) => {
    set((s) => {
      if (!(path in s.conflicts)) return s;
      const next = { ...s.conflicts };
      delete next[path];
      return { conflicts: next };
    });
  },

}));

attachSiblingStores({ editor: useEditorStore });

import { create } from "zustand";

import {
  readFile,
  writeFile,
  type WorkspaceChangeKind,
} from "../api/workspace";
import {
  BINARY_FILE_MESSAGE,
  fileKindFromPath,
  isTextKind,
  type FileKind,
} from "../lib/fileKind";
import { flushMarkdownEditor } from "../lib/markdownFlush";
import {
  WorkspaceRequestError,
  undisplayableStatus,
} from "../lib/workspaceError";
import { languageFromPath, fileNameFromPath } from "../utils/language";
import {
  isWysiwygMarkdownPath,
  type MdEditorView,
} from "../utils/wysiwygMarkdown";
import { remapPathPrefix } from "../utils/path";
import { closingFlags } from "../dockview/config/sharedFlags";
import { closePanel, openPanel, revealPanel } from "../dockview/workbench/commands";
import { onPanelRemoved } from "../dockview/workbench/events";
import { attachSiblingStores } from "./connectionStore";

/** Tab id for an external preview. The chip stores the path; the bytes live on the tab. */
export function externalPreviewId(path: string): string {
  return `external:${path}`;
}

export interface EditorTab {
  path: string;
  content: string;
  savedContent: string;
  dirty: boolean;
  language: string;
  loading: boolean;
  error: string | null;
  /** Network or connection failure. Display errors are not retried. */
  errorRetryable: boolean;
  kind: FileKind;
  /** Bumped when a non-text file changes on disk so its preview refetches. */
  diskRevision: number;
  /** Dropped from outside the workspace. Not read or written through the workspace API. */
  external?: boolean;
  /** Object URL for an external image, pdf, or media preview. */
  previewUrl?: string;
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
  pendingReveal: { path: string; line: number; column?: number } | null;
  /** Per-tab Markdown view. Missing means default (wysiwyg for `.md`). */
  mdViewByPath: Record<string, MdEditorView>;
  jumpBack: JumpLocation[];
  jumpForward: JumpLocation[];

  openFile: (path: string) => Promise<void>;
  /** Load or refresh a tab. Safe to call on every connect, including reconnects. */
  ensureReadable: (path: string) => Promise<void>;
  /** Open file and reveal a 1-based line (workspace search / go-to). */
  openFileAt: (path: string, line: number, column?: number) => Promise<void>;
  consumePendingReveal: () => {
    path: string;
    line: number;
    column?: number;
  } | null;
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
  setMdView: (path: string, view: MdEditorView) => void;
  /** Temporary preview of a file dropped from outside the workspace. */
  openExternalPreview: (file: File, chip: string) => Promise<void>;
}

function makeTab(path: string, content: string, diskRevision = 0): EditorTab {
  return {
    path,
    content,
    savedContent: content,
    dirty: false,
    language: languageFromPath(path),
    loading: false,
    error: null,
    errorRetryable: false,
    kind: "text",
    diskRevision,
  };
}

function shellTab(path: string, kind: FileKind, diskRevision = 0): EditorTab {
  return {
    path,
    content: "",
    savedContent: "",
    dirty: false,
    language: languageFromPath(path),
    loading: kind === "text",
    error: kind === "binary" ? BINARY_FILE_MESSAGE : null,
    errorRetryable: false,
    kind,
    diskRevision,
  };
}

function failureTab(path: string, error: unknown, diskRevision: number): EditorTab {
  const message = error instanceof Error ? error.message : String(error);
  if (error instanceof WorkspaceRequestError && undisplayableStatus(error.status)) {
    return {
      ...shellTab(path, "binary", diskRevision),
      loading: false,
      error: message,
    };
  }
  const retryable =
    !(error instanceof WorkspaceRequestError) || error.retryable;
  return {
    ...shellTab(path, "text", diskRevision),
    loading: false,
    error: message,
    errorRetryable: retryable,
  };
}

const readableInflight = new Map<string, Promise<void>>();

async function loadReadable(
  path: string,
  get: () => EditorStore,
  set: (
    partial:
      | Partial<EditorStore>
      | ((state: EditorStore) => Partial<EditorStore>),
  ) => void,
): Promise<void> {
  const existing = get().tabs.find((t) => t.path === path);
  if (existing?.dirty) return;
  if (existing?.kind === "binary" && existing.error && !existing.errorRetryable) {
    return;
  }

  const kind = fileKindFromPath(path);
  if (!isTextKind(kind)) {
    if (!existing) {
      set((s) => ({ tabs: [...s.tabs, shellTab(path, kind)] }));
      return;
    }
    if (existing.kind !== kind) {
      set((s) => ({
        tabs: s.tabs.map((t) =>
          t.path === path
            ? {
                ...shellTab(path, kind, t.diskRevision),
                diskRevision: t.diskRevision + 1,
              }
            : t,
        ),
      }));
      return;
    }
    if (existing.errorRetryable) {
      set((s) => ({
        tabs: s.tabs.map((t) =>
          t.path === path
            ? {
                ...t,
                error: null,
                errorRetryable: false,
                diskRevision: t.diskRevision + 1,
              }
            : t,
        ),
      }));
    }
    return;
  }

  // A permanent text failure (missing file, and so on) has no buffer to keep.
  if (
    existing?.kind === "text" &&
    existing.error &&
    !existing.errorRetryable &&
    existing.content === ""
  ) {
    return;
  }

  const loaded =
    !!existing &&
    existing.kind === "text" &&
    !existing.error &&
    !existing.loading;
  if (!loaded) {
    set((s) => {
      const current = s.tabs.find((t) => t.path === path);
      if (current?.dirty) return s;
      if (!current) return { tabs: [...s.tabs, shellTab(path, "text")] };
      return {
        tabs: s.tabs.map((t) =>
          t.path === path ? { ...t, loading: true, error: null } : t,
        ),
      };
    });
  }

  if (get().tabs.find((t) => t.path === path)?.dirty) return;

  try {
    const content = await readFile(path);
    const current = get().tabs.find((t) => t.path === path);
    if (!current || current.dirty) {
      if (current?.loading) {
        set((s) => ({
          tabs: s.tabs.map((t) =>
            t.path === path ? { ...t, loading: false } : t,
          ),
        }));
      }
      return;
    }
    if (
      current.kind === "text" &&
      !current.loading &&
      !current.error &&
      current.content === content &&
      current.savedContent === content
    ) {
      return;
    }
    const revision = current.diskRevision;
    set((s) => ({
      tabs: s.tabs.map((t) =>
        t.path === path ? makeTab(path, content, revision) : t,
      ),
    }));
  } catch (error) {
    const current = get().tabs.find((t) => t.path === path);
    if (!current || current.dirty) return;
    const failed = failureTab(path, error, current.diskRevision);
    set((s) => ({
      tabs: s.tabs.map((t) => (t.path === path ? failed : t)),
    }));
  }
}

function openEditorPanel(path: string): void {
  openPanel({
    id: path,
    component: "editor",
    title: fileNameFromPath(path),
    tabComponent: "editor",
    params: { filePath: path },
  });
}

export const useEditorStore = create<EditorStore>((set, get) => ({
  tabs: [],
  conflicts: {},
  activePath: null,
  saving: false,
  pendingReveal: null,
  mdViewByPath: {},
  jumpBack: [],
  jumpForward: [],

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
    openEditorPanel(path);
    const existing = get().tabs.find((t) => t.path === path);
    set({ activePath: path });
    if (existing && (existing.dirty || !existing.errorRetryable)) return;
    await get().ensureReadable(path);
  },

  ensureReadable: (path) => {
    if (get().tabs.find((tab) => tab.path === path)?.external) {
      return Promise.resolve();
    }
    const pending = readableInflight.get(path);
    if (pending) return pending;
    const job = loadReadable(path, get, set).finally(() => {
      if (readableInflight.get(path) === job) readableInflight.delete(path);
    });
    readableInflight.set(path, job);
    return job;
  },

  closeTab: (path) => {
    const leaving = get().tabs.find((tab) => tab.path === path);
    if (leaving?.previewUrl) URL.revokeObjectURL(leaving.previewUrl);
    closingFlags.closingFromStore = true;
    try {
      closePanel(path);
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
    revealPanel(path);
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
    if (!tab || tab.external || tab.loading || !isTextKind(tab.kind)) return;

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
                  errorRetryable: false,
                }
              : t,
          ),
        };
      });
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      set((s) => ({
        saving: false,
        tabs: s.tabs.map((t) => (t.path === path ? { ...t, error: msg } : t)),
      }));
    }
  },

  reloadFromDisk: async (path) => {
    const tab = get().tabs.find((t) => t.path === path);
    if (!tab || tab.external) return;
    if (!isTextKind(tab.kind)) {
      set((s) => ({
        tabs: s.tabs.map((t) =>
          t.path === path ? { ...t, diskRevision: t.diskRevision + 1 } : t,
        ),
      }));
      return;
    }
    try {
      const content = await readFile(path);
      const current = get().tabs.find((t) => t.path === path);
      if (!current) return;
      if (
        current.kind === "text" &&
        !current.dirty &&
        !current.error &&
        current.content === content &&
        current.savedContent === content
      ) {
        return;
      }
      const revision = current.diskRevision;
      set((s) => ({
        tabs: s.tabs.map((t) =>
          t.path === path ? makeTab(path, content, revision) : t,
        ),
      }));
    } catch (error) {
      const current = get().tabs.find((t) => t.path === path);
      const revision = current?.diskRevision ?? 0;
      const failed = failureTab(path, error, revision);
      set((s) => ({
        tabs: s.tabs.map((t) => (t.path === path ? failed : t)),
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
      if (!tab || tab.external) continue;

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
    const { tabs } = get();
    const affected = tabs.filter(
      (t) =>
        !t.external &&
        (t.path === from || (from !== "" && t.path.startsWith(`${from}/`))),
    );
    if (affected.length === 0) return;

    const becameText = affected
      .filter((t) => {
        const next = remapPathPrefix(t.path, from, to);
        return (
          next !== t.path &&
          isTextKind(fileKindFromPath(next)) &&
          !isTextKind(t.kind)
        );
      })
      .map((t) => remapPathPrefix(t.path, from, to));

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
          const kind = fileKindFromPath(path);
          const next = { ...t, path, language: languageFromPath(path), kind };
          if (kind === t.kind) return next;
          if (!isTextKind(kind)) {
            return {
              ...shellTab(path, kind, t.diskRevision),
              diskRevision: t.diskRevision + 1,
            };
          }
          return {
            ...next,
            loading: true,
            error: null,
            errorRetryable: false,
            content: "",
            savedContent: "",
            dirty: false,
          };
        }),
        activePath: s.activePath
          ? remapPathPrefix(s.activePath, from, to)
          : null,
        conflicts: nextConflicts,
        mdViewByPath,
      };
    });

    for (const path of becameText) {
      void get().reloadFromDisk(path);
    }

    for (const tab of affected) {
      const oldPath = tab.path;
      const newPath = remapPathPrefix(oldPath, from, to);
      if (oldPath === newPath) continue;
      closingFlags.closingFromStore = true;
      try {
        closePanel(oldPath);
      } finally {
        closingFlags.closingFromStore = false;
      }
      openEditorPanel(newPath);
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

  openExternalPreview: async (file, chip) => {
    const id = externalPreviewId(chip);
    const named = file.name || fileNameFromPath(chip);
    const kind = fileKindFromPath(named);
    const previous = get().tabs.find((tab) => tab.path === id);
    if (previous?.previewUrl) URL.revokeObjectURL(previous.previewUrl);

    let next: EditorTab;
    if (isTextKind(kind)) {
      const content = await file.text();
      next = { ...makeTab(id, content), external: true };
    } else if (
      kind === "image" ||
      kind === "pdf" ||
      kind === "audio" ||
      kind === "video"
    ) {
      next = {
        ...shellTab(id, kind),
        loading: false,
        error: null,
        external: true,
        previewUrl: URL.createObjectURL(file),
      };
    } else {
      next = { ...shellTab(id, "binary"), external: true };
    }

    set((state) => {
      const exists = state.tabs.some((tab) => tab.path === id);
      return {
        activePath: id,
        tabs: exists
          ? state.tabs.map((tab) => (tab.path === id ? next : tab))
          : [...state.tabs, next],
      };
    });

    openPanel({
      id,
      component: "editor",
      title: fileNameFromPath(named),
      tabComponent: "editor",
      params: { filePath: id },
    });
  },
}));

onPanelRemoved((event) => {
  if (event.component !== "editor" || closingFlags.closingFromStore) return;
  useEditorStore.getState().closeTab(event.id);
});

attachSiblingStores({ editor: useEditorStore });

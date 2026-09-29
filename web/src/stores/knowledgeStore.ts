import { create } from "zustand";

import { replaceKnowledgeKey } from "../lib/knowledge/document";
import { isKnowledgeKey, normalizeKey } from "../lib/knowledge/markers";
import {
  createKnowledgeFolder,
  createKnowledgeNode,
  deleteKnowledgeEntry,
  KNOWLEDGE_PRIVATE_ROOT,
  KNOWLEDGE_PUBLIC_ROOT,
  knowledgeDiskPath,
  knowledgeFolderRel,
  knowledgeMoveDestination,
  knowledgeNodeRel,
  knowledgeRootExists,
  loadKnowledgeFromWorkspace,
  moveKnowledgeEntry,
  moveKnowledgeRoot,
  readKnowledgeSnapshot,
  writeKnowledgeNode,
  type KnowledgeVisibility,
} from "../lib/knowledge/load";
import type {
  KnowledgeFolder,
  KnowledgeIssue,
  KnowledgeNode,
} from "../lib/knowledge/types";
import { groupIssues, validateKnowledge } from "../lib/knowledge/validate";
import { useWorkspaceChangeStore } from "./workspaceChangeStore";

function indexNodes(nodes: KnowledgeNode[]) {
  const byId = new Map<string, KnowledgeNode>();
  const byKey = new Map<string, KnowledgeNode>();
  for (const node of nodes) {
    if (!byId.has(node.id)) byId.set(node.id, node);
    const key = normalizeKey(node.key);
    if (key && !byKey.has(key)) byKey.set(key, node);
  }
  const issues = validateKnowledge(nodes);
  return { nodes, byId, byKey, issues, issuesByNode: groupIssues(issues) };
}

function indexFolders(folders: KnowledgeFolder[]) {
  const folderById = new Map<string, KnowledgeFolder>();
  for (const folder of folders) {
    if (!folderById.has(folder.id)) folderById.set(folder.id, folder);
  }
  return { folders, folderById };
}

export function knowledgeSnapshot(
  nodes: KnowledgeNode[],
  folders: KnowledgeFolder[],
) {
  return {
    ...indexNodes(nodes),
    ...indexFolders(folders),
    expandedFolders: new Set(folders.map((folder) => folder.id)),
  };
}

export type KnowledgeNodePatch = Partial<
  Pick<
    KnowledgeNode,
    "summary" | "value" | "relations" | "status" | "x" | "y" | "w" | "h"
  >
>;

export type KnowledgeRenameResult = "ok" | "invalid" | "duplicate";

export type KnowledgeCreateResult = KnowledgeRenameResult | "error";

function sameNode(a: KnowledgeNode, b: KnowledgeNode): boolean {
  return (
    a.key === b.key &&
    a.summary === b.summary &&
    a.value === b.value &&
    a.status === b.status &&
    (a.invalidStatus ?? null) === (b.invalidStatus ?? null) &&
    a.x === b.x &&
    a.y === b.y &&
    a.w === b.w &&
    a.h === b.h &&
    a.relations.join("\0") === b.relations.join("\0") &&
    a.id === b.id
  );
}

interface KnowledgeStore {
  nodes: KnowledgeNode[];
  byId: Map<string, KnowledgeNode>;
  byKey: Map<string, KnowledgeNode>;
  folders: KnowledgeFolder[];
  folderById: Map<string, KnowledgeFolder>;
  issues: KnowledgeIssue[];
  issuesByNode: Map<string, KnowledgeIssue[]>;
  expanded: Set<string>;
  expandedFolders: Set<string>;
  graphExpanded: Set<string>;
  focusedId: string | null;
  /** Bumps when the canvas should pan to `focusedId` (side body / ref jump). */
  focusNonce: number;
  /** Side-list flash target when a graph card is clicked. */
  flashId: string | null;
  flashNonce: number;
  /** Bumps when folders or files move, so the canvas drops stale positions. */
  structureNonce: number;
  /** Directory that currently holds the corpus. */
  root: string;
  visibility: KnowledgeVisibility;
  loading: boolean;
  error: string | null;
  /** Read the public or private tree, seeding private when both are missing. */
  load: () => Promise<void>;
  /**
   * Re-read the corpus from disk. Used when a knowledge panel becomes active.
   * Keeps expansion, focus, and canvas positions unless files or folders moved.
   */
  refreshFromDisk: () => Promise<void>;
  /** Write one node's declaration block and body. */
  saveNode: (id: string, patch: KnowledgeNodePatch) => Promise<void>;
  /**
   * Rename a declaration. Rewrites mention `id`s, and `label`s that equal the
   * old declaration, in the other files. Rejects a key that is illegal or
   * already declared.
   */
  renameNode: (id: string, nextKey: string) => Promise<KnowledgeRenameResult>;
  /** Create a directory under `.litecode/knowledge`. `parentId` null is the root. */
  createFolder: (
    parentId: string | null,
    name: string,
  ) => Promise<KnowledgeCreateResult>;
  /** Create `key.md` and its `node` declaration in that folder. */
  createNode: (
    folderId: string | null,
    key: string,
  ) => Promise<KnowledgeCreateResult>;
  /** Delete one markdown file. Other files keep their mention shortcodes. */
  deleteNode: (id: string) => Promise<void>;
  /** Delete a directory and the nodes inside it. */
  deleteFolder: (id: string) => Promise<void>;
  /** Move a node md into another folder. `folderId` null is the corpus root. */
  moveNode: (
    id: string,
    folderId: string | null,
  ) => Promise<KnowledgeCreateResult>;
  /** Move a folder under another folder. `parentId` null is the corpus root. */
  moveFolder: (
    id: string,
    parentId: string | null,
  ) => Promise<KnowledgeCreateResult>;
  /** Physically move the corpus between `.litecode/knowledge` and `knowledge/`. */
  setVisibility: (next: KnowledgeVisibility) => Promise<void>;
  /** Pan the graph to this node; does not expand the side row. */
  focusCanvas: (id: string) => void;
  /** Graph card click: highlight graph + flash side without expanding or panning. */
  selectFromGraph: (id: string) => void;
  /** Click graph background — drop canvas/side focus highlight. */
  clearCanvasFocus: () => void;
  toggle: (id: string) => void;
  toggleFolder: (id: string) => void;
  toggleGraph: (id: string) => void;
  expandAll: () => void;
  collapseAll: () => void;
}

const empty = knowledgeSnapshot([], []);

function diskError(err: unknown): string {
  let msg = err instanceof Error ? err.message : String(err);
  try {
    const parsed = JSON.parse(msg) as { error?: string };
    if (parsed.error) msg = parsed.error;
  } catch {
    /* keep the raw message */
  }
  return msg;
}

function isConflict(err: unknown): boolean {
  return /already exists|HTTP 409/i.test(diskError(err));
}

function pathTaken(
  rel: string,
  nodes: KnowledgeNode[],
  folders: KnowledgeFolder[],
): boolean {
  const needle = rel.toLowerCase();
  return (
    folders.some((folder) => folder.id.toLowerCase() === needle) ||
    nodes.some((node) => node.path.toLowerCase() === needle)
  );
}

function aliveIds(ids: Iterable<string>, alive: Set<string>): Set<string> {
  return new Set([...ids].filter((id) => alive.has(id)));
}

async function applyDisk(
  set: (partial: Partial<KnowledgeStore>) => void,
  get: () => KnowledgeStore,
): Promise<void> {
  const state = get();
  const loaded = await readKnowledgeSnapshot(state.root);
  const nodeIds = new Set(loaded.nodes.map((node) => node.id));
  const folderIds = new Set(loaded.folders.map((folder) => folder.id));
  set({
    ...knowledgeSnapshot(loaded.nodes, loaded.folders),
    expanded: aliveIds(state.expanded, nodeIds),
    expandedFolders: aliveIds(state.expandedFolders, folderIds),
    graphExpanded: aliveIds(state.graphExpanded, nodeIds),
    focusedId:
      state.focusedId != null && nodeIds.has(state.focusedId)
        ? state.focusedId
        : null,
    focusNonce: state.focusNonce,
    flashId:
      state.flashId != null && nodeIds.has(state.flashId) ? state.flashId : null,
    flashNonce: state.flashNonce,
    structureNonce: state.structureNonce + 1,
    loading: false,
    error: null,
  });
}

function folderContains(
  ancestorId: string,
  maybeChild: string | null,
  folders: KnowledgeFolder[],
): boolean {
  const byId = new Map(folders.map((folder) => [folder.id, folder]));
  let current = maybeChild;
  const seen = new Set<string>();
  while (current && !seen.has(current)) {
    if (current === ancestorId) return true;
    seen.add(current);
    current = byId.get(current)?.parentId ?? null;
  }
  return false;
}

async function refreshExplorer(rel: string): Promise<void> {
  const [{ useTreeStore }, { parentPath }] = await Promise.all([
    import("./treeStore"),
    import("../utils/path"),
  ]);
  const disk = knowledgeDiskPath(useKnowledgeStore.getState().root, rel);
  const tree = useTreeStore.getState();
  const keys = new Set<string>(["", parentPath(disk), disk]);
  for (const key of keys) tree.invalidate(key);
  const reloads = [...keys].filter(
    (key) => key === "" || useTreeStore.getState().expanded.has(key),
  );
  await Promise.all(
    reloads.map((key) => useTreeStore.getState().loadChildren(key)),
  );
}

async function closeEditor(rel: string): Promise<void> {
  const { useEditorStore } = await import("./editorStore");
  useEditorStore
    .getState()
    .closeDeleted(knowledgeDiskPath(useKnowledgeStore.getState().root, rel));
}

let loadInflight: Promise<void> | null = null;
let refreshInflight: Promise<void> | null = null;
let hydrated = false;

function sameCorpus(
  nodes: KnowledgeNode[],
  folders: KnowledgeFolder[],
  root: string,
  loaded: {
    nodes: KnowledgeNode[];
    folders: KnowledgeFolder[];
    root: string;
  },
): boolean {
  if (root !== loaded.root) return false;
  if (nodes.length !== loaded.nodes.length) return false;
  if (folders.length !== loaded.folders.length) return false;
  for (let i = 0; i < folders.length; i++) {
    const current = folders[i];
    const next = loaded.folders[i];
    if (
      current.id !== next.id ||
      current.name !== next.name ||
      current.parentId !== next.parentId
    ) {
      return false;
    }
  }
  for (let i = 0; i < nodes.length; i++) {
    const current = nodes[i];
    const next = loaded.nodes[i];
    if (current.path !== next.path) return false;
    if ((current.folderId ?? null) !== (next.folderId ?? null)) return false;
    if (!sameNode(current, next)) return false;
  }
  return true;
}

function structureChanged(
  nodes: KnowledgeNode[],
  folders: KnowledgeFolder[],
  loaded: { nodes: KnowledgeNode[]; folders: KnowledgeFolder[] },
): boolean {
  if (nodes.length !== loaded.nodes.length) return true;
  if (folders.length !== loaded.folders.length) return true;
  for (let i = 0; i < folders.length; i++) {
    if (folders[i].id !== loaded.folders[i].id) return true;
    if (folders[i].parentId !== loaded.folders[i].parentId) return true;
  }
  for (let i = 0; i < nodes.length; i++) {
    if (nodes[i].id !== loaded.nodes[i].id) return true;
    if (nodes[i].path !== loaded.nodes[i].path) return true;
    if ((nodes[i].folderId ?? null) !== (loaded.nodes[i].folderId ?? null)) {
      return true;
    }
  }
  return false;
}

export const useKnowledgeStore = create<KnowledgeStore>((set, get) => ({
  ...empty,
  expanded: new Set<string>(),
  graphExpanded: new Set<string>(),
  root: KNOWLEDGE_PRIVATE_ROOT,
  visibility: "private",
  structureNonce: 0,
  focusedId: null,
  focusNonce: 0,
  flashId: null,
  flashNonce: 0,
  loading: false,
  error: null,
  load: () => {
    if (loadInflight) return loadInflight;
    loadInflight = (async () => {
      set({ loading: true, error: null });
      try {
        const loaded = await loadKnowledgeFromWorkspace();
        hydrated = true;
        set({
          ...knowledgeSnapshot(loaded.nodes, loaded.folders),
          root: loaded.root,
          visibility: loaded.visibility,
          structureNonce: get().structureNonce + 1,
          loading: false,
          error: null,
        });
      } catch (err) {
        const message = err instanceof Error ? err.message : "无法读取知识库";
        set({ loading: false, error: message });
      } finally {
        loadInflight = null;
      }
    })();
    return loadInflight;
  },
  refreshFromDisk: () => {
    if (!hydrated || loadInflight) return get().load();
    if (refreshInflight) return refreshInflight;
    refreshInflight = (async () => {
      try {
        const loaded = await loadKnowledgeFromWorkspace();
        const latest = get();
        if (sameCorpus(latest.nodes, latest.folders, latest.root, loaded)) {
          return;
        }
        const nodeIds = new Set(loaded.nodes.map((node) => node.id));
        const folderIds = new Set(loaded.folders.map((folder) => folder.id));
        const moved = structureChanged(latest.nodes, latest.folders, loaded);
        set({
          ...knowledgeSnapshot(loaded.nodes, loaded.folders),
          expanded: aliveIds(latest.expanded, nodeIds),
          expandedFolders: aliveIds(latest.expandedFolders, folderIds),
          graphExpanded: aliveIds(latest.graphExpanded, nodeIds),
          focusedId:
            latest.focusedId != null && nodeIds.has(latest.focusedId)
              ? latest.focusedId
              : null,
          focusNonce: latest.focusNonce,
          flashId:
            latest.flashId != null && nodeIds.has(latest.flashId)
              ? latest.flashId
              : null,
          flashNonce: latest.flashNonce,
          structureNonce: moved
            ? latest.structureNonce + 1
            : latest.structureNonce,
          root: loaded.root,
          visibility: loaded.visibility,
          loading: false,
          error: null,
        });
      } catch (err) {
        set({ error: diskError(err) });
      } finally {
        refreshInflight = null;
      }
    })();
    return refreshInflight;
  },
  saveNode: async (id, patch) => {
    const current = get().byId.get(id);
    if (!current) return;
    const next = {
      ...current,
      ...patch,
      ...(patch.status !== undefined ? { invalidStatus: null } : {}),
    };
    if (sameNode(current, next)) return;
    const nodes = get().nodes.map((node) => (node.id === id ? next : node));
    set((state) => ({
      ...knowledgeSnapshot(nodes, state.folders),
      expanded: state.expanded,
      expandedFolders: state.expandedFolders,
      graphExpanded: state.graphExpanded,
      focusedId: state.focusedId,
      focusNonce: state.focusNonce,
      flashId: state.flashId,
      flashNonce: state.flashNonce,
      loading: state.loading,
      error: null,
    }));
    try {
      await writeKnowledgeNode(next, get().root);
    } catch (err) {
      const message = err instanceof Error ? err.message : "无法写入知识库";
      set({ error: message });
    }
  },
  renameNode: async (id, rawKey) => {
    const state = get();
    const current = state.byId.get(id);
    if (!current) return "invalid";
    const nextKey = normalizeKey(rawKey);
    const from = normalizeKey(current.key);
    if (nextKey === from) return "ok";
    if (!isKnowledgeKey(nextKey)) return "invalid";
    const taken = state.nodes.some(
      (node) => node.id !== id && normalizeKey(node.key) === nextKey,
    );
    if (taken) return "duplicate";
    const nextId = current.id === from ? nextKey : current.id;
    const nodes = state.nodes.map((node) => {
      const renamed = node.id === id;
      const relations = node.relations.map((ref) => (ref === from ? nextKey : ref));
      const value = replaceKnowledgeKey(node.value, from, nextKey);
      if (!renamed && relations.join("\0") === node.relations.join("\0") && value === node.value) {
        return node;
      }
      return {
        ...node,
        id: renamed ? nextId : node.id,
        key: renamed ? nextKey : node.key,
        relations,
        value,
      };
    });
    const changed = nodes.filter((node, index) => node !== state.nodes[index]);
    const remap = (value: string) => (value === id ? nextId : value);
    set({
      ...knowledgeSnapshot(nodes, state.folders),
      expanded: new Set([...state.expanded].map(remap)),
      expandedFolders: state.expandedFolders,
      graphExpanded: new Set([...state.graphExpanded].map(remap)),
      focusedId: state.focusedId == null ? null : remap(state.focusedId),
      focusNonce: state.focusNonce,
      flashId: state.flashId == null ? null : remap(state.flashId),
      flashNonce: state.flashNonce,
      loading: state.loading,
      error: null,
    });
    try {
      await Promise.all(
        changed.map((node) => writeKnowledgeNode(node, get().root)),
      );
    } catch (err) {
      const message = err instanceof Error ? err.message : "无法写入知识库";
      set({ error: message });
    }
    return "ok";
  },
  createFolder: async (parentId, name) => {
    const state = get();
    if (parentId != null && !state.folderById.has(parentId)) return "invalid";
    const rel = knowledgeFolderRel(parentId, name);
    if (!rel) return "invalid";
    if (pathTaken(rel, state.nodes, state.folders)) return "duplicate";
    try {
      await createKnowledgeFolder(state.root, rel);
      await refreshExplorer(rel).catch(() => undefined);
      const loaded = await readKnowledgeSnapshot(state.root);
      const nodeIds = new Set(loaded.nodes.map((node) => node.id));
      const folderIds = new Set(loaded.folders.map((folder) => folder.id));
      const expandedFolders = aliveIds(state.expandedFolders, folderIds);
      if (parentId) expandedFolders.add(parentId);
      expandedFolders.add(rel);
      set({
        ...knowledgeSnapshot(loaded.nodes, loaded.folders),
        expanded: aliveIds(state.expanded, nodeIds),
        expandedFolders,
        graphExpanded: aliveIds(state.graphExpanded, nodeIds),
        focusedId:
          state.focusedId != null && nodeIds.has(state.focusedId)
            ? state.focusedId
            : null,
        focusNonce: state.focusNonce,
        flashId:
          state.flashId != null && nodeIds.has(state.flashId)
            ? state.flashId
            : null,
        flashNonce: state.flashNonce,
        structureNonce: state.structureNonce + 1,
        loading: false,
        error: null,
      });
      return "ok";
    } catch (err) {
      if (isConflict(err)) return "duplicate";
      set({ error: diskError(err) });
      return "error";
    }
  },
  createNode: async (folderId, rawKey) => {
    const state = get();
    if (folderId != null && !state.folderById.has(folderId)) return "invalid";
    const key = normalizeKey(rawKey);
    const rel = knowledgeNodeRel(folderId, key);
    if (!rel) return "invalid";
    if (state.byKey.has(key) || pathTaken(rel, state.nodes, state.folders)) {
      return "duplicate";
    }
    try {
      await createKnowledgeNode(state.root, rel, key);
      await refreshExplorer(rel).catch(() => undefined);
      const loaded = await readKnowledgeSnapshot(state.root);
      const created = loaded.nodes.find((node) => node.path === rel);
      const nodeIds = new Set(loaded.nodes.map((node) => node.id));
      const folderIds = new Set(loaded.folders.map((folder) => folder.id));
      const expandedFolders = aliveIds(state.expandedFolders, folderIds);
      if (folderId) expandedFolders.add(folderId);
      const focusedId = created && nodeIds.has(created.id) ? created.id : null;
      set({
        ...knowledgeSnapshot(loaded.nodes, loaded.folders),
        expanded: aliveIds(state.expanded, nodeIds),
        expandedFolders,
        graphExpanded: aliveIds(state.graphExpanded, nodeIds),
        focusedId,
        focusNonce: focusedId ? state.focusNonce + 1 : state.focusNonce,
        flashId: focusedId,
        flashNonce: state.flashNonce + 1,
        structureNonce: state.structureNonce + 1,
        loading: false,
        error: null,
      });
      return "ok";
    } catch (err) {
      if (isConflict(err)) return "duplicate";
      set({ error: diskError(err) });
      return "error";
    }
  },
  deleteNode: async (id) => {
    const current = get().byId.get(id);
    if (!current) return;
    try {
      await deleteKnowledgeEntry(get().root, current.path, false);
      await closeEditor(current.path);
      await refreshExplorer(current.path).catch(() => undefined);
      await applyDisk(set, get);
    } catch (err) {
      set({ error: diskError(err) });
    }
  },
  deleteFolder: async (id) => {
    if (!get().folderById.has(id)) return;
    try {
      await deleteKnowledgeEntry(get().root, id, true);
      await closeEditor(id);
      await refreshExplorer(id).catch(() => undefined);
      await applyDisk(set, get);
    } catch (err) {
      set({ error: diskError(err) });
    }
  },
  moveNode: async (id, folderId) => {
    const state = get();
    const current = state.byId.get(id);
    if (!current) return "invalid";
    if ((current.folderId ?? null) === folderId) return "ok";
    if (folderId != null && !state.folderById.has(folderId)) return "invalid";
    const dest = knowledgeMoveDestination(current.path, folderId);
    if (dest === current.path) return "ok";
    if (pathTaken(dest, state.nodes, state.folders)) return "duplicate";
    try {
      await moveKnowledgeEntry(state.root, current.path, dest);
      await writeKnowledgeNode(
        { ...current, path: dest, folderId, x: null, y: null },
        state.root,
      );
      const { useEditorStore } = await import("./editorStore");
      useEditorStore
        .getState()
        .remapTabs(
          knowledgeDiskPath(state.root, current.path),
          knowledgeDiskPath(state.root, dest),
        );
      await refreshExplorer(dest).catch(() => undefined);
      await applyDisk(set, get);
      return "ok";
    } catch (err) {
      if (isConflict(err)) return "duplicate";
      set({ error: diskError(err) });
      return "error";
    }
  },
  moveFolder: async (id, parentId) => {
    const state = get();
    const folder = state.folderById.get(id);
    if (!folder) return "invalid";
    if ((folder.parentId ?? null) === parentId) return "ok";
    if (parentId != null && !state.folderById.has(parentId)) return "invalid";
    if (folderContains(id, parentId, state.folders)) return "invalid";
    const dest = knowledgeMoveDestination(id, parentId);
    if (dest === id) return "ok";
    if (pathTaken(dest, state.nodes, state.folders)) return "duplicate";
    try {
      await moveKnowledgeEntry(state.root, id, dest);
      const { useEditorStore } = await import("./editorStore");
      useEditorStore
        .getState()
        .remapTabs(
          knowledgeDiskPath(state.root, id),
          knowledgeDiskPath(state.root, dest),
        );
      await refreshExplorer(dest).catch(() => undefined);
      const loaded = await readKnowledgeSnapshot(state.root);
      const folderIds = new Set(loaded.folders.map((item) => item.id));
      const nodeIds = new Set(loaded.nodes.map((node) => node.id));
      const remap = (value: string) =>
        value === id || value.startsWith(`${id}/`)
          ? `${dest}${value.slice(id.length)}`
          : value;
      const focusedId =
        state.focusedId == null ? null : remap(state.focusedId);
      const flashId = state.flashId == null ? null : remap(state.flashId);
      set({
        ...knowledgeSnapshot(loaded.nodes, loaded.folders),
        expanded: aliveIds([...state.expanded].map(remap), nodeIds),
        expandedFolders: aliveIds(
          [...state.expandedFolders].map(remap),
          folderIds,
        ),
        graphExpanded: aliveIds([...state.graphExpanded].map(remap), nodeIds),
        focusedId: focusedId != null && nodeIds.has(focusedId) ? focusedId : null,
        focusNonce: state.focusNonce,
        flashId: flashId != null && nodeIds.has(flashId) ? flashId : null,
        flashNonce: state.flashNonce,
        structureNonce: state.structureNonce + 1,
        loading: false,
        error: null,
      });
      return "ok";
    } catch (err) {
      if (isConflict(err)) return "duplicate";
      set({ error: diskError(err) });
      return "error";
    }
  },
  setVisibility: async (next) => {
    const state = get();
    if (state.visibility === next) return;
    const to =
      next === "public" ? KNOWLEDGE_PUBLIC_ROOT : KNOWLEDGE_PRIVATE_ROOT;
    try {
      if (await knowledgeRootExists(to)) {
        set({ error: "另一处已经有知识库，没有移动" });
        return;
      }
      await moveKnowledgeRoot(state.root, to);
      const { useEditorStore } = await import("./editorStore");
      useEditorStore.getState().remapTabs(state.root, to);
      const { useTreeStore } = await import("./treeStore");
      useTreeStore.getState().invalidate(state.root);
      set({ root: to, visibility: next });
      await refreshExplorer("").catch(() => undefined);
      await applyDisk(set, get);
    } catch (err) {
      set({ error: diskError(err) });
    }
  },
  focusCanvas: (id) =>
    set((state) => {
      if (!state.byId.has(id)) return state;
      return {
        focusedId: id,
        focusNonce: state.focusNonce + 1,
      };
    }),
  selectFromGraph: (id) =>
    set((state) => {
      if (!state.byId.has(id)) return state;
      return {
        focusedId: id,
        flashId: id,
        flashNonce: state.flashNonce + 1,
      };
    }),
  clearCanvasFocus: () =>
    set((state) => {
      if (state.focusedId == null) return state;
      return { focusedId: null, flashId: null };
    }),
  toggle: (id) =>
    set((state) => {
      const expanded = new Set(state.expanded);
      if (expanded.has(id)) expanded.delete(id);
      else expanded.add(id);
      return { expanded };
    }),
  toggleFolder: (id) =>
    set((state) => {
      if (!state.folderById.has(id)) return state;
      const expandedFolders = new Set(state.expandedFolders);
      if (expandedFolders.has(id)) expandedFolders.delete(id);
      else expandedFolders.add(id);
      return { expandedFolders };
    }),
  toggleGraph: (id) =>
    set((state) => {
      if (!state.byId.has(id)) return state;
      const graphExpanded = new Set(state.graphExpanded);
      if (graphExpanded.has(id)) graphExpanded.delete(id);
      else graphExpanded.add(id);
      return { graphExpanded };
    }),
  expandAll: () =>
    set((state) => ({
      expanded: new Set(state.nodes.map((node) => node.id)),
    })),
  collapseAll: () => set({ expanded: new Set() }),
}));

function pathUnderKnowledgeRoot(path: string, root: string): boolean {
  const norm = path.replaceAll("\\", "/");
  return norm === root || norm.startsWith(`${root}/`);
}

let knowledgeRefreshTimer: ReturnType<typeof setTimeout> | null = null;

useWorkspaceChangeStore.subscribe((state, prev) => {
  if (!state.last || state.last === prev.last) return;
  const root = useKnowledgeStore.getState().root;
  if (!state.last.paths.some((path) => pathUnderKnowledgeRoot(path, root))) return;
  if (knowledgeRefreshTimer) clearTimeout(knowledgeRefreshTimer);
  knowledgeRefreshTimer = setTimeout(() => {
    knowledgeRefreshTimer = null;
    void useKnowledgeStore.getState().refreshFromDisk();
  }, 150);
});

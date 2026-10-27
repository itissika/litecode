import { create } from "zustand";

import {
  fetchKnowledgeSnapshot,
  type KnowledgeChanged,
  type WireKnowledgeNode,
  type WireKnowledgeSnapshot,
} from "../api/knowledge";
import { replaceKnowledgeKey } from "../lib/knowledge/document";
import { extractRefs, isKnowledgeKey, normalizeKey } from "../lib/knowledge/markers";
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
  moveKnowledgeEntry,
  moveKnowledgeRoot,
  writeKnowledgeNode,
  type KnowledgeVisibility,
} from "../lib/knowledge/load";
import {
  acceptsKnowledgeVersion,
  knowledgeViewFromWire,
  reuseRefIndex,
  type KnowledgeRefIndex,
  type KnowledgeView,
} from "../lib/knowledge/snapshot";
import type {
  KnowledgeFolder,
  KnowledgeIssue,
  KnowledgeNode,
  KnowledgeRefState,
  KnowledgeUnknownFile,
} from "../lib/knowledge/types";
import { replaceOneRecord, reuseIssueGroups } from "../lib/knowledge/flowProjection";
import { groupIssues } from "../lib/knowledge/validate";
import i18n from "../i18n";

/*
 * The knowledge panel consumes the backend snapshot
 * (`GET /api/workspace/knowledge` + `knowledge/changed`,
 * `docs/knowledge-references.md` §10–11). Corpus membership, folders, and
 * every citation verdict come from there; this store never walks the tree,
 * stats a cited path, or asks `/symbol-refs`. Local writes (explicit user
 * saves) show at once and stay overlaid until a snapshot carries them.
 */

const NO_REFS: ReadonlyMap<string, KnowledgeRefState> = new Map();

function indexNodes(nodes: KnowledgeNode[], issues: KnowledgeIssue[]) {
  const byId = new Map<string, KnowledgeNode>();
  const byKey = new Map<string, KnowledgeNode>();
  for (const node of nodes) {
    if (!byId.has(node.id)) byId.set(node.id, node);
    const key = normalizeKey(node.key);
    if (key && !byKey.has(key)) byKey.set(key, node);
  }
  return { nodes, byId, byKey, issues, issuesByNode: groupIssues(issues) };
}

function indexFolders(folders: KnowledgeFolder[]) {
  const folderById = new Map<string, KnowledgeFolder>();
  for (const folder of folders) {
    if (!folderById.has(folder.id)) folderById.set(folder.id, folder);
  }
  return { folders, folderById };
}

/**
 * Indexed store slice for a list of nodes and folders. `issues` and
 * `refsByNode` are backend verdicts (empty when there is no snapshot, as in
 * tests and before the first load).
 */
export function knowledgeSnapshot(
  nodes: KnowledgeNode[],
  folders: KnowledgeFolder[],
  issues: KnowledgeIssue[] = [],
  refsByNode: KnowledgeRefIndex = new Map(),
) {
  return {
    ...indexNodes(nodes, issues),
    ...indexFolders(folders),
    refsByNode,
    expandedFolders: new Set(folders.map((folder) => folder.id)),
  };
}

/** `ref_key`s cited in a body, local extraction (same formula as the backend). */
function localRefKeys(value: string): Set<string> {
  return new Set(extractRefs(value).map((ref) => ref.ref_key));
}

/**
 * Issues of a node the user just edited, before the backend has seen the edit:
 * a citation that is no longer in the body loses its issue at once; nothing
 * new is judged locally (new citations stay "checking" until the snapshot).
 */
function pruneEditedIssues(
  issues: KnowledgeIssue[],
  node: KnowledgeNode,
  statusPatched: boolean,
): KnowledgeIssue[] {
  let keys: Set<string> | null = null;
  return issues.filter((issue) => {
    if (issue.nodeId !== node.id) return true;
    if (statusPatched && issue.code === "invalid_status") return false;
    if (!issue.refKey) return true;
    keys ??= localRefKeys(node.value);
    return keys.has(issue.refKey);
  });
}

/** Reindex one local corpus edit. Unchanged issue lists keep their arrays. */
export function projectListedNodes(
  previous: {
    nodes: KnowledgeNode[];
    issues: KnowledgeIssue[];
    issuesByNode: Map<string, KnowledgeIssue[]>;
    folders: KnowledgeFolder[];
  },
  listed: KnowledgeNode[],
  edited: { node: KnowledgeNode; statusPatched: boolean }[] = [],
) {
  let issues = previous.issues;
  for (const edit of edited) issues = pruneEditedIssues(issues, edit.node, edit.statusPatched);
  const indexed = indexNodes(listed, issues);
  return {
    ...indexed,
    issuesByNode: reuseIssueGroups(previous.issuesByNode, indexed.issuesByNode),
  };
}

type KnowledgeNodePatch = Partial<
  Pick<
    KnowledgeNode,
    "summary" | "value" | "relations" | "status" | "x" | "y" | "w" | "h"
  >
>;

type KnowledgeRenameResult = "ok" | "invalid" | "duplicate";

type KnowledgeCreateResult = KnowledgeRenameResult | "error";

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
  /** Backend verdict per node id, per `ref_key`. */
  refsByNode: KnowledgeRefIndex;
  /** Snapshot `ready`. False: citations show "checking", never an error. */
  ready: boolean;
  /** Snapshot `version` / `epoch` last applied (accept rule, §10.2). */
  version: number;
  epoch: string;
  /** Markdown files with no declaration. Shown in the side list, not on the canvas. */
  unknown: KnowledgeUnknownFile[];
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
  /** First read of the backend snapshot. Later calls share or reuse it. */
  load: () => Promise<void>;
  /**
   * Dockview visibility for one knowledge surface (side list or canvas).
   * The first surface to become visible pulls a fresh snapshot.
   */
  notePanelVisible: (panelId: string, visible: boolean) => Promise<void>;
  /**
   * Pull the backend snapshot again.
   * Keeps expansion, focus, and canvas positions unless files or folders moved.
   */
  refreshFromDisk: () => Promise<void>;
  /** `knowledge/changed` push: debounced refetch when the version is new. */
  onKnowledgeChanged: (params: KnowledgeChanged) => void;
  /** WS (re)connected: pushes may have been missed, so refetch unconditionally. */
  onReconnect: () => void;
  /**
   * Write one node's declaration block and body.
   * Resolves `false` when the write fails, so the caller can keep the unsaved mark.
   */
  saveNode: (id: string, patch: KnowledgeNodePatch) => Promise<boolean>;
  /**
   * Write `x,y,w,h` for one node. Does not touch issues or bump
   * `structureNonce`. Other node objects keep their references.
   */
  saveGeometry: (
    id: string,
    patch: Partial<Pick<KnowledgeNode, "x" | "y" | "w" | "h">>,
  ) => Promise<boolean>;
  /**
   * Rename a declaration. Rewrites mention `id`s, and `label`s that equal the
   * old declaration, in the other files. Rejects a key that is illegal or
   * already declared.
   */
  renameNode: (id: string, nextKey: string) => Promise<KnowledgeRenameResult>;
  /** Create a directory under the knowledge root. `parentId` null is the root. */
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
  /**
   * Locate a node in the side list: expand its ancestor folders and flash the
   * row. Document mode calls this as the reader moves between nodes.
   */
  revealInList: (id: string) => void;
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

// ------------------------------------------------------------ local overlay

/**
 * Nodes this client wrote that the backend has not reported back yet, by
 * node path. A snapshot that still has the old content (it was taken before
 * the watcher saw the write) cannot undo a save on screen.
 */
interface PendingWrite {
  node: KnowledgeNode;
  at: number;
}

/** After this long, the snapshot wins even if it never matched (external edit). */
const PENDING_WRITE_MS = 5000;

const pendingWrites = new Map<string, PendingWrite>();

function notePendingWrite(node: KnowledgeNode): void {
  pendingWrites.set(node.path, { node, at: Date.now() });
}

function dropPendingWrite(path: string, node: KnowledgeNode): void {
  if (pendingWrites.get(path)?.node === node) pendingWrites.delete(path);
}

function round(value: number | null): number | null {
  return value == null || !Number.isFinite(value) ? null : Math.round(value);
}

function writtenMatches(wire: WireKnowledgeNode, node: KnowledgeNode): boolean {
  return (
    normalizeKey(wire.key) === normalizeKey(node.key) &&
    (wire.body ?? "").trimEnd() === node.value.trimEnd() &&
    (wire.summary ?? "").trim() === node.summary.trim() &&
    wire.status === node.status &&
    (wire.invalid_status ?? null) === (node.invalidStatus ?? null) &&
    round(wire.x) === round(node.x) &&
    round(wire.y) === round(node.y) &&
    round(wire.w) === round(node.w) &&
    round(wire.h) === round(node.h)
  );
}

/** Lay pending local writes over a snapshot, and retire the ones it now carries. */
function overlayPendingWrites(wire: WireKnowledgeSnapshot): WireKnowledgeSnapshot {
  if (pendingWrites.size === 0) return wire;
  const now = Date.now();
  let changed = false;
  const nodes = (wire.nodes ?? []).map((node) => {
    const pending = pendingWrites.get(node.path);
    if (!pending) return node;
    if (writtenMatches(node, pending.node) || now - pending.at > PENDING_WRITE_MS) {
      pendingWrites.delete(node.path);
      return node;
    }
    changed = true;
    // Verdicts on citations the pending body no longer has are stale.
    const keys = localRefKeys(pending.node.value);
    const statusChanged = node.status !== pending.node.status;
    return {
      ...node,
      issues: (node.issues ?? []).filter(
        (issue) =>
          !(statusChanged && issue.code === "invalid_status") &&
          (issue.ref_key == null || keys.has(issue.ref_key)),
      ),
      refs: (node.refs ?? []).filter((ref) => keys.has(ref.ref_key)),
      key: pending.node.key,
      body: pending.node.value,
      summary: pending.node.summary,
      status: pending.node.status,
      invalid_status: pending.node.invalidStatus ?? null,
      x: pending.node.x,
      y: pending.node.y,
      w: pending.node.w,
      h: pending.node.h,
    };
  });
  for (const [path, pending] of pendingWrites) {
    if (now - pending.at > PENDING_WRITE_MS) pendingWrites.delete(path);
  }
  return changed ? { ...wire, nodes } : wire;
}

// ------------------------------------------------------------ snapshot sync

let hydrated = false;
let loadInflight: Promise<void> | null = null;
let refreshInflight: Promise<void> | null = null;
let refreshAgain = false;
let changeTimer: ReturnType<typeof setTimeout> | null = null;
let changeWaiters: Array<() => void> = [];

/** Quiet period between a `knowledge/changed` push and the refetch. */
const KNOWLEDGE_PUSH_DEBOUNCE_MS = 60;

function wakeChangeWaiters(): void {
  const waiters = changeWaiters;
  changeWaiters = [];
  for (const wake of waiters) wake();
}

/** Resolves on the next push, or after `ms` (polling fallback when the WS is down). */
function nextChangeOr(ms: number): Promise<void> {
  return new Promise((resolve) => {
    const timer = setTimeout(() => {
      changeWaiters = changeWaiters.filter((wake) => wake !== done);
      resolve();
    }, ms);
    const done = () => {
      clearTimeout(timer);
      resolve();
    };
    changeWaiters.push(done);
  });
}

type SetState = (partial: Partial<KnowledgeStore>) => void;
type GetState = () => KnowledgeStore;

function sameShape(a: KnowledgeNode, b: KnowledgeNode): boolean {
  return (
    sameNode(a, b) &&
    a.path === b.path &&
    (a.folderId ?? null) === (b.folderId ?? null) &&
    (a.extras ?? []).join("\n") === (b.extras ?? []).join("\n")
  );
}

/** Reuse unchanged node objects so cards that did not change do not re-render. */
function reuseNodes(previous: Map<string, KnowledgeNode>, next: KnowledgeNode[]): KnowledgeNode[] {
  return next.map((node) => {
    const prior = previous.get(node.id);
    return prior && sameShape(prior, node) ? prior : node;
  });
}

/**
 * Apply one snapshot. Returns false when the accept rule rejects it: an older
 * version of the same epoch (§10.2). Re-applying the current version is
 * allowed (it retires local overlays the snapshot now carries).
 */
function applyView(set: SetState, get: GetState, view: KnowledgeView): boolean {
  const state = get();
  if (
    hydrated &&
    view.epoch === state.epoch &&
    view.version < state.version
  ) {
    return false;
  }
  const first = !hydrated;
  hydrated = true;
  const nodes = reuseNodes(state.byId, view.nodes);
  const nodeIds = new Set(nodes.map((node) => node.id));
  const folderIds = new Set(view.folders.map((folder) => folder.id));
  const moved = first || structureChanged(state.nodes, state.folders, view);
  const indexed = indexNodes(nodes, view.issues);
  set({
    ...indexed,
    ...indexFolders(view.folders),
    issuesByNode: reuseIssueGroups(state.issuesByNode, indexed.issuesByNode),
    refsByNode: reuseRefIndex(state.refsByNode, view.refsByNode),
    ready: view.ready,
    version: view.version,
    epoch: view.epoch,
    unknown: view.unknown,
    root: view.root,
    visibility: view.visibility,
    expanded: aliveIds(state.expanded, nodeIds),
    expandedFolders: first
      ? new Set(view.folders.map((folder) => folder.id))
      : aliveIds(state.expandedFolders, folderIds),
    graphExpanded: aliveIds(state.graphExpanded, nodeIds),
    focusedId:
      state.focusedId != null && nodeIds.has(state.focusedId) ? state.focusedId : null,
    flashId: state.flashId != null && nodeIds.has(state.flashId) ? state.flashId : null,
    structureNonce: moved ? state.structureNonce + 1 : state.structureNonce,
    loading: false,
    error: null,
  });
  return true;
}

async function pullSnapshot(set: SetState, get: GetState): Promise<void> {
  const wire = await fetchKnowledgeSnapshot();
  applyView(set, get, knowledgeViewFromWire(overlayPendingWrites(wire)));
}

/** One refetch at a time; a request during a fetch runs once more after it. */
function refresh(set: SetState, get: GetState): Promise<void> {
  if (refreshInflight) {
    refreshAgain = true;
    return refreshInflight;
  }
  refreshInflight = (async () => {
    try {
      do {
        refreshAgain = false;
        try {
          await pullSnapshot(set, get);
        } catch (err) {
          // Keep the last snapshot; never judge locally (F5).
          set({ error: diskError(err) });
        }
      } while (refreshAgain);
    } finally {
      refreshInflight = null;
    }
  })();
  return refreshInflight;
}

/**
 * After a structural write, refetch until the snapshot shows it (the backend
 * sees disk changes through its watcher). Gives up after `timeoutMs`; the next
 * push still brings the result.
 */
async function settle(
  set: SetState,
  get: GetState,
  done: (state: KnowledgeStore) => boolean,
  timeoutMs = 3000,
): Promise<boolean> {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    await refresh(set, get);
    if (done(get())) return true;
    if (Date.now() >= deadline) return false;
    await nextChangeOr(150);
  }
}

function setLocalNodes(
  set: SetState,
  state: KnowledgeStore,
  listed: KnowledgeNode[],
  edited: { node: KnowledgeNode; statusPatched: boolean }[],
  message: string | null,
) {
  set({ ...projectListedNodes(state, listed, edited), error: message });
}

export const useKnowledgeStore = create<KnowledgeStore>((set, get) => ({
  ...empty,
  ready: false,
  version: 0,
  epoch: "",
  expanded: new Set<string>(),
  graphExpanded: new Set<string>(),
  root: KNOWLEDGE_PRIVATE_ROOT,
  visibility: "private",
  unknown: [],
  structureNonce: 0,
  focusedId: null,
  focusNonce: 0,
  flashId: null,
  flashNonce: 0,
  loading: false,
  error: null,
  load: () => {
    if (hydrated) return refreshInflight ?? Promise.resolve();
    if (loadInflight) return loadInflight;
    loadInflight = (async () => {
      set({ loading: true, error: null });
      try {
        await pullSnapshot(set, get);
      } catch (err) {
        const message = err instanceof Error ? err.message : i18n.t("knowledge.readFailed");
        set({ loading: false, error: message });
      } finally {
        loadInflight = null;
      }
    })();
    return loadInflight;
  },
  refreshFromDisk: () => {
    if (!hydrated) return get().load();
    return refresh(set, get);
  },
  onKnowledgeChanged: (params) => {
    wakeChangeWaiters();
    if (!hydrated) return;
    const state = get();
    if (!acceptsKnowledgeVersion({ version: state.version, epoch: state.epoch }, params)) {
      return;
    }
    if (changeTimer) clearTimeout(changeTimer);
    changeTimer = setTimeout(() => {
      changeTimer = null;
      void refresh(set, get);
    }, KNOWLEDGE_PUSH_DEBOUNCE_MS);
  },
  onReconnect: () => {
    if (!hydrated) return;
    void refresh(set, get);
  },
  saveNode: async (id, patch) => {
    const current = get().byId.get(id);
    if (!current) return false;
    const statusPatched = patch.status !== undefined;
    const next = {
      ...current,
      ...patch,
      ...(statusPatched ? { invalidStatus: null } : {}),
    };
    if (sameNode(current, next)) return true;
    notePendingWrite(next);
    const nodes = get().nodes.map((node) => (node.id === id ? next : node));
    setLocalNodes(set, get(), nodes, [{ node: next, statusPatched }], null);
    try {
      await writeKnowledgeNode(next, get().root);
    } catch (err) {
      dropPendingWrite(next.path, next);
      const message = err instanceof Error ? err.message : i18n.t("knowledge.writeFailed");
      const state = get();
      const restored = state.nodes.map((node) => (node.id === id ? current : node));
      setLocalNodes(set, state, restored, [], message);
      return false;
    }
    return true;
  },
  saveGeometry: async (id, patch) => {
    const current = get().byId.get(id);
    if (!current) return false;
    const next = { ...current, ...patch };
    if (sameNode(current, next)) return true;
    notePendingWrite(next);
    const state = get();
    const replaced = replaceOneRecord(state.nodes, state.byId, next);
    set({ nodes: replaced.records, byId: replaced.index, error: null });
    try {
      await writeKnowledgeNode(next, get().root);
    } catch (err) {
      dropPendingWrite(next.path, next);
      const message = err instanceof Error ? err.message : i18n.t("knowledge.writeFailed");
      const latest = get();
      if (latest.byId.get(id) === next) {
        const restored = replaceOneRecord(latest.nodes, latest.byId, current);
        set({ nodes: restored.records, byId: restored.index, error: message });
      } else {
        set({ error: message });
      }
      return false;
    }
    return true;
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
    for (const node of changed) notePendingWrite(node);
    const remap = (value: string) => (value === id ? nextId : value);
    // Issues keyed by the old id move with the node; the backend re-judges the rename.
    const issues = state.issues.map((issue) =>
      issue.nodeId === id ? { ...issue, nodeId: nextId } : issue,
    );
    const refsByNode: KnowledgeRefIndex = new Map(
      [...state.refsByNode].map(([nodeId, refs]) => [remap(nodeId), refs]),
    );
    set({
      ...indexNodes(nodes, issues),
      refsByNode,
      expanded: new Set([...state.expanded].map(remap)),
      graphExpanded: new Set([...state.graphExpanded].map(remap)),
      focusedId: state.focusedId == null ? null : remap(state.focusedId),
      flashId: state.flashId == null ? null : remap(state.flashId),
      error: null,
    });
    try {
      await Promise.all(
        changed.map((node) => writeKnowledgeNode(node, get().root)),
      );
    } catch (err) {
      for (const node of changed) dropPendingWrite(node.path, node);
      const message = err instanceof Error ? err.message : i18n.t("knowledge.writeFailed");
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
      await settle(set, get, (s) => s.folderById.has(rel));
      const expandedFolders = new Set(get().expandedFolders);
      if (parentId) expandedFolders.add(parentId);
      expandedFolders.add(rel);
      set({ expandedFolders });
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
      await settle(set, get, (s) => s.nodes.some((node) => node.path === rel));
      const latest = get();
      const created = latest.nodes.find((node) => node.path === rel);
      const expandedFolders = new Set(latest.expandedFolders);
      if (folderId) expandedFolders.add(folderId);
      const focusedId = created ? created.id : null;
      set({
        expandedFolders,
        focusedId,
        focusNonce: focusedId ? latest.focusNonce + 1 : latest.focusNonce,
        flashId: focusedId,
        flashNonce: latest.flashNonce + 1,
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
      await settle(set, get, (s) => !s.nodes.some((node) => node.path === current.path));
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
      await settle(set, get, (s) => !s.folderById.has(id));
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
      await settle(set, get, (s) => s.nodes.some((node) => node.path === dest));
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
      await settle(set, get, (s) => s.folderById.has(dest) && !s.folderById.has(id));
      const latest = get();
      const folderIds = new Set(latest.folders.map((item) => item.id));
      const nodeIds = new Set(latest.nodes.map((node) => node.id));
      const remap = (value: string) =>
        value === id || value.startsWith(`${id}/`)
          ? `${dest}${value.slice(id.length)}`
          : value;
      const focusedId = state.focusedId == null ? null : remap(state.focusedId);
      const flashId = state.flashId == null ? null : remap(state.flashId);
      set({
        expanded: aliveIds([...state.expanded].map(remap), nodeIds),
        expandedFolders: aliveIds([...state.expandedFolders].map(remap), folderIds),
        graphExpanded: aliveIds([...state.graphExpanded].map(remap), nodeIds),
        focusedId: focusedId != null && nodeIds.has(focusedId) ? focusedId : null,
        flashId: flashId != null && nodeIds.has(flashId) ? flashId : null,
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
      await moveKnowledgeRoot(state.root, to);
    } catch (err) {
      set({
        error: isConflict(err) ? i18n.t("knowledge.moveConflict") : diskError(err),
      });
      return;
    }
    try {
      const { useEditorStore } = await import("./editorStore");
      useEditorStore.getState().remapTabs(state.root, to);
      const { useTreeStore } = await import("./treeStore");
      useTreeStore.getState().invalidate(state.root);
      set({ root: to, visibility: next });
      await refreshExplorer("").catch(() => undefined);
      await settle(set, get, (s) => s.root === to);
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
  revealInList: (id) =>
    set((state) => {
      const node = state.byId.get(id);
      if (!node) return state;
      const expandedFolders = new Set(state.expandedFolders);
      const seen = new Set<string>();
      let folderId = node.folderId;
      while (folderId != null && !seen.has(folderId)) {
        seen.add(folderId);
        expandedFolders.add(folderId);
        folderId = state.folderById.get(folderId)?.parentId ?? null;
      }
      return {
        expandedFolders,
        flashId: id,
        flashNonce: state.flashNonce + 1,
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
  notePanelVisible: (panelId, visible) => {
    const wasOpen = knowledgeVisiblePanels.size > 0;
    if (visible) knowledgeVisiblePanels.add(panelId);
    else knowledgeVisiblePanels.delete(panelId);
    const open = knowledgeVisiblePanels.size > 0;
    if (open && !wasOpen) return get().refreshFromDisk();
    return Promise.resolve();
  },
}));

const knowledgeVisiblePanels = new Set<string>();

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

/** Backend verdict for one citation of one node, or undefined while unknown. */
export function knowledgeRefState(
  state: Pick<KnowledgeStore, "refsByNode">,
  nodeId: string | undefined | null,
  refKey: string,
): KnowledgeRefState | undefined {
  if (!nodeId) return undefined;
  return (state.refsByNode.get(nodeId) ?? NO_REFS).get(refKey);
}

/** Test hook: forget hydration, overlays, and timers between cases. */
export function resetKnowledgeSyncForTests(): void {
  hydrated = false;
  loadInflight = null;
  refreshInflight = null;
  refreshAgain = false;
  if (changeTimer) clearTimeout(changeTimer);
  changeTimer = null;
  changeWaiters = [];
  pendingWrites.clear();
  knowledgeVisiblePanels.clear();
}

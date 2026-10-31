import type { WireKnowledgeNode, WireKnowledgeSnapshot } from "../../api/knowledge";
import { KNOWLEDGE_ROOT } from "./document";
import { isKnowledgeKey, normalizeKey } from "./markers";
import type {
  KnowledgeFolder,
  KnowledgeIssue,
  KnowledgeNode,
  KnowledgeRefState,
  KnowledgeRefStatus,
  KnowledgeUnknownFile,
} from "./types";

export type KnowledgeVisibility = "private" | "public";

/** Per node id: backend verdict per `ref_key`. */
export type KnowledgeRefIndex = Map<string, ReadonlyMap<string, KnowledgeRefState>>;

/** The backend snapshot in the shapes the panel already renders. */
export interface KnowledgeView {
  version: number;
  epoch: string;
  ready: boolean;
  root: string;
  visibility: KnowledgeVisibility;
  nodes: KnowledgeNode[];
  folders: KnowledgeFolder[];
  unknown: KnowledgeUnknownFile[];
  issues: KnowledgeIssue[];
  refsByNode: KnowledgeRefIndex;
}

/**
 * Frontend node ids, the rule `knowledgeFromFiles` has always used, so
 * expansion, focus, and canvas state keep their keys: a unique legal key is the
 * id; an illegal or duplicate key (or an id already taken) uses the path.
 * Node identity on the wire is `path`; this id is derived from it.
 */
export function knowledgeNodeIds(
  nodes: readonly { path: string; key: string }[],
): string[] {
  const keyCounts = new Map<string, number>();
  for (const node of nodes) {
    const key = normalizeKey(node.key);
    if (!isKnowledgeKey(key)) continue;
    keyCounts.set(key, (keyCounts.get(key) ?? 0) + 1);
  }
  const used = new Set<string>();
  return nodes.map((node) => {
    const key = normalizeKey(node.key);
    const unique = isKnowledgeKey(key) && keyCounts.get(key) === 1;
    let id = unique ? key : node.path;
    if (used.has(id)) id = node.path;
    used.add(id);
    return id;
  });
}

/**
 * Accept rule for snapshots and pushes (§9.2): a larger version, or any
 * version from a different epoch (the server restarted).
 */
export function acceptsKnowledgeVersion(
  current: { version: number; epoch: string } | null,
  next: { version: number; epoch: string },
): boolean {
  if (!current) return true;
  if (next.epoch !== current.epoch) return true;
  return next.version > current.version;
}

/** Legacy `ref` display field: what the problem is about. */
function legacyRef(refKey: string | null, args: Record<string, unknown>): string | undefined {
  const pick = (name: string) => {
    const value = args[name];
    return typeof value === "string" ? value : undefined;
  };
  if (refKey?.startsWith("node:")) return pick("key") ?? refKey.slice(5);
  if (refKey?.startsWith("sym:")) return pick("symbol");
  if (refKey) return pick("path");
  return pick("key") ?? pick("status");
}

function nodeIssues(wire: WireKnowledgeNode, nodeId: string): KnowledgeIssue[] {
  const out: KnowledgeIssue[] = [];
  for (const issue of wire.issues ?? []) {
    // Unknown future codes still show: severity drives tone, message the text.
    const code = issue.code as KnowledgeIssue["code"];
    const args = issue.args ?? {};
    const refKey = issue.ref_key ?? null;
    out.push({
      nodeId,
      severity: issue.severity,
      code,
      message: issue.message ?? "",
      ref: legacyRef(refKey, args),
      refKey,
      args,
    });
  }
  return out;
}

function nodeRefs(wire: WireKnowledgeNode): Map<string, KnowledgeRefState> {
  const refs = new Map<string, KnowledgeRefState>();
  for (const ref of wire.refs ?? []) {
    if (refs.has(ref.ref_key)) continue;
    refs.set(ref.ref_key, {
      refKey: ref.ref_key,
      kind: ref.kind,
      key: ref.key ?? null,
      path: ref.path ?? null,
      symbol: ref.symbol ?? null,
      lines: ref.lines ?? null,
      raw: ref.raw ?? "",
      status: ref.status as KnowledgeRefStatus,
      severity: ref.severity ?? null,
      targetPath: ref.target_path ?? null,
      args: ref.args ?? {},
      message: ref.message ?? "",
    });
  }
  return refs;
}

export function knowledgeViewFromWire(wire: WireKnowledgeSnapshot): KnowledgeView {
  const wireNodes = wire.nodes ?? [];
  const ids = knowledgeNodeIds(wireNodes);
  const nodes: KnowledgeNode[] = [];
  const issues: KnowledgeIssue[] = [];
  const refsByNode: KnowledgeRefIndex = new Map();
  wireNodes.forEach((wireNode, index) => {
    const id = ids[index]!;
    nodes.push({
      id,
      key: normalizeKey(wireNode.key),
      value: wireNode.body ?? "",
      summary: wireNode.summary ?? "",
      relations: wireNode.relations ?? [],
      status: wireNode.status,
      invalidStatus: wireNode.invalid_status ?? null,
      x: wireNode.x ?? null,
      y: wireNode.y ?? null,
      w: wireNode.w ?? null,
      h: wireNode.h ?? null,
      folderId: wireNode.folder_id ?? null,
      path: wireNode.path,
      extras: wireNode.extras ?? [],
    });
    issues.push(...nodeIssues(wireNode, id));
    if (!refsByNode.has(id)) refsByNode.set(id, nodeRefs(wireNode));
  });
  return {
    version: wire.version,
    epoch: wire.epoch,
    ready: wire.ready,
    root: wire.root ?? KNOWLEDGE_ROOT,
    visibility: wire.visibility === "public" ? "public" : "private",
    nodes,
    folders: (wire.folders ?? []).map((folder) => ({
      id: folder.id,
      name: folder.name,
      parentId: folder.parent_id ?? null,
    })),
    unknown: (wire.unknown ?? []).map((file) => ({
      path: file.path,
      name: file.name,
      folderId: file.folder_id ?? null,
    })),
    issues,
    refsByNode,
  };
}

function sameRefState(a: KnowledgeRefState, b: KnowledgeRefState): boolean {
  return (
    a.status === b.status &&
    a.severity === b.severity &&
    a.targetPath === b.targetPath &&
    a.message === b.message &&
    a.raw === b.raw &&
    JSON.stringify(a.args) === JSON.stringify(b.args)
  );
}

/** Keep per-node ref maps whose verdicts did not change, so chips do not re-render. */
export function reuseRefIndex(
  previous: KnowledgeRefIndex,
  next: KnowledgeRefIndex,
): KnowledgeRefIndex {
  const out: KnowledgeRefIndex = new Map();
  for (const [id, refs] of next) {
    const prior = previous.get(id);
    if (!prior || prior.size !== refs.size) {
      out.set(id, refs);
      continue;
    }
    let same = true;
    const merged = new Map<string, KnowledgeRefState>();
    for (const [key, ref] of refs) {
      const old = prior.get(key);
      if (old && sameRefState(old, ref)) merged.set(key, old);
      else {
        same = false;
        merged.set(key, ref);
      }
    }
    out.set(id, same ? prior : merged);
  }
  return out;
}

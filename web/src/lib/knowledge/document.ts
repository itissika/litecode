import { extractMarkers, isKnowledgeKey, knowledgePreview, normalizeKey } from "./markers";
import type {
  KnowledgeFolder,
  KnowledgeNode,
  KnowledgeStatus,
} from "./types";

/** Workspace-relative root for on-disk knowledge nodes. */
export const KNOWLEDGE_ROOT = ".litecode/knowledge";

export interface KnowledgeSourceFile {
  /** Path relative to `.litecode/knowledge`, forward slashes. */
  path: string;
  markdown: string;
}

export interface ParsedKnowledgeFile {
  path: string;
  key: string;
  status: KnowledgeStatus;
  summary: string;
  /** Declared `ref :` keys, in source order. */
  refs: string[];
  /** Body after the declaration fence. The fence itself is not included. */
  body: string;
  folderId: string | null;
  x: number | null;
  y: number | null;
  w: number | null;
  h: number | null;
  /** True when the fence contains a `summary` or `ref` line. */
  hasMeta: boolean;
}

const OPEN_FENCE = /^(?:[ \t]*\r?\n)*```node[ \t]*\r?\n/;
const CLOSE_FENCE = /\r?\n```[ \t]*(?:\r?\n|$)/;
const FIELD_LINE = /^([A-Za-z_][A-Za-z0-9_-]*)\s*:\s*(.*?)\s*$/;

function normalizeRel(path: string): string {
  return path.replaceAll("\\", "/").replace(/^\/+/, "").replace(/\/+$/, "");
}

function folderIdOf(filePath: string): string | null {
  const slash = filePath.lastIndexOf("/");
  if (slash <= 0) return null;
  return filePath.slice(0, slash);
}

function parseStatus(value: string): KnowledgeStatus | null {
  if (value === "enabled" || value === "disabled" || value === "pending") {
    return value;
  }
  return null;
}

function parseCoord(value: string): number | null {
  if (!value) return null;
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : null;
}

function pushRef(refs: string[], value: string): void {
  const key = normalizeKey(value);
  if (!isKnowledgeKey(key) || refs.includes(key)) return;
  refs.push(key);
}

/**
 * Read a node file. The declaration is a `node` fence at the start of the file.
 * Anything before that fence means the file has no declaration.
 */
export function parseKnowledgeMarkdown(
  path: string,
  markdown: string,
): ParsedKnowledgeFile {
  const normalizedPath = normalizeRel(path);
  const source = markdown.replace(/^\uFEFF/, "");
  const open = OPEN_FENCE.exec(source);
  let key = "";
  let status: KnowledgeStatus = "enabled";
  let summary = "";
  const refs: string[] = [];
  let body = source;
  let hasMeta = false;
  let x: number | null = null;
  let y: number | null = null;
  let w: number | null = null;
  let h: number | null = null;
  if (open) {
    const afterOpen = source.slice(open[0].length);
    const close = CLOSE_FENCE.exec(afterOpen);
    if (close) {
      const block = afterOpen.slice(0, close.index);
      body = afterOpen.slice(close.index + close[0].length).replace(/^(?:\r?\n)/, "");
      let sawNode = false;
      for (const line of block.split(/\r?\n/)) {
        const trimmed = line.trim();
        if (!trimmed) continue;
        const field = FIELD_LINE.exec(trimmed);
        if (!field) continue;
        const name = field[1] ?? "";
        const value = (field[2] ?? "").trim();
        if (name === "node" && !sawNode) {
          sawNode = true;
          key = value;
        } else if (name === "status") {
          const parsed = parseStatus(value);
          if (parsed) status = parsed;
        } else if (name === "summary") {
          hasMeta = true;
          summary = value;
        } else if (name === "ref") {
          hasMeta = true;
          pushRef(refs, value);
        } else if (name === "x") x = parseCoord(value);
        else if (name === "y") y = parseCoord(value);
        else if (name === "w") w = parseCoord(value);
        else if (name === "h") h = parseCoord(value);
      }
    }
  }
  return {
    path: normalizedPath,
    key,
    status,
    summary,
    refs,
    body,
    folderId: folderIdOf(normalizedPath),
    x,
    y,
    w,
    h,
    hasMeta,
  };
}

function coordLine(name: string, value: number | null | undefined): string | null {
  if (value == null || !Number.isFinite(value)) return null;
  return `${name} : ${Math.round(value)}`;
}

/** Serialize a node back to the on-disk markdown shape. */
export function renderKnowledgeMarkdown(doc: {
  key: string;
  status: KnowledgeStatus;
  body: string;
  summary?: string;
  refs?: string[];
  x?: number | null;
  y?: number | null;
  w?: number | null;
  h?: number | null;
}): string {
  const body = doc.body.replace(/\s+$/, "");
  const lines = [
    "```node",
    `node : ${doc.key}`,
    `status : ${doc.status}`,
    `summary : ${doc.summary ?? ""}`,
    ...(doc.refs ?? []).map((ref) => `ref : ${ref}`),
  ];
  for (const line of [
    coordLine("x", doc.x),
    coordLine("y", doc.y),
    coordLine("w", doc.w),
    coordLine("h", doc.h),
  ]) {
    if (line) lines.push(line);
  }
  lines.push("```", "", body, "");
  return lines.join("\n");
}

/**
 * Files written before summary and ref lines existed get those lines once.
 * A fence that already has either field is left unchanged.
 */
export function upgradeKnowledgeMarkdown(markdown: string): string | null {
  const parsed = parseKnowledgeMarkdown("node.md", markdown);
  if (!parsed.key || parsed.hasMeta) return null;
  const refs: string[] = [];
  for (const marker of extractMarkers(parsed.body)) pushRef(refs, marker);
  return renderKnowledgeMarkdown({
    key: normalizeKey(parsed.key),
    status: parsed.status,
    summary: knowledgePreview(parsed.body, 1),
    refs,
    body: parsed.body,
    x: parsed.x,
    y: parsed.y,
    w: parsed.w,
    h: parsed.h,
  });
}

const MARKER_KEY = /\[\[\s*node\s*:\s*([^\]\r\n]+?)\s*\]\]/g;

/** Rewrite `[[node : from]]` markers. Declaration lines are left alone. */
export function replaceKnowledgeKey(text: string, from: string, to: string): string {
  const source = normalizeKey(from);
  const target = normalizeKey(to);
  if (!source || source === target) return text;
  return text.replace(MARKER_KEY, (full, key: string) =>
    key === source ? `[[node : ${target}]]` : full,
  );
}

/** `ref : key` lines, one per declared reference. */
export function formatRefBlock(refs: string[]): string {
  return refs.map((ref) => `ref : ${ref}`).join("\n");
}

/** Keep complete `ref : key` lines. Incomplete lines are ignored. */
export function parseRefBlock(text: string): string[] {
  const refs: string[] = [];
  for (const line of text.split(/\r?\n/)) {
    const field = /^\s*ref\s*:\s*(.+?)\s*$/.exec(line);
    const key = field?.[1];
    if (key) pushRef(refs, key);
  }
  return refs;
}

function addFolderChain(ids: Set<string>, dir: string): void {
  const norm = normalizeRel(dir);
  if (!norm || norm.toLowerCase().endsWith(".md")) return;
  const parts = norm.split("/");
  for (let i = 1; i <= parts.length; i += 1) {
    ids.add(parts.slice(0, i).join("/"));
  }
}

function foldersFromIds(ids: Set<string>): KnowledgeFolder[] {
  return [...ids]
    .sort((a, b) => a.localeCompare(b))
    .map((id) => {
      const slash = id.lastIndexOf("/");
      return {
        id,
        name: slash === -1 ? id : id.slice(slash + 1),
        parentId: slash === -1 ? null : id.slice(0, slash),
      };
    });
}

/**
 * Turn source files into nodes and folders.
 * A unique legal key becomes the node id. A missing, illegal, or duplicate key
 * keeps the file path as the id so both copies can still render.
 */
export function knowledgeFromFiles(
  files: KnowledgeSourceFile[],
  directories: string[] = [],
): { nodes: KnowledgeNode[]; folders: KnowledgeFolder[] } {
  const parsed = files.map((file) =>
    parseKnowledgeMarkdown(file.path, file.markdown),
  );
  const keyCounts = new Map<string, number>();
  for (const file of parsed) {
    const key = normalizeKey(file.key);
    if (!isKnowledgeKey(key)) continue;
    keyCounts.set(key, (keyCounts.get(key) ?? 0) + 1);
  }

  const usedIds = new Set<string>();
  const nodes: KnowledgeNode[] = parsed.map((file) => {
    const key = normalizeKey(file.key);
    const unique = isKnowledgeKey(key) && keyCounts.get(key) === 1;
    let id = unique ? key : file.path;
    if (usedIds.has(id)) id = file.path;
    usedIds.add(id);
    return {
      id,
      key,
      value: file.body,
      summary: file.summary,
      relations: file.refs,
      status: file.status,
      folderId: file.folderId,
      path: file.path,
      x: file.x,
      y: file.y,
      w: file.w,
      h: file.h,
    };
  });

  const folderIds = new Set<string>();
  for (const dir of directories) addFolderChain(folderIds, dir);
  for (const node of nodes) {
    if (node.folderId) addFolderChain(folderIds, node.folderId);
  }
  return { nodes, folders: foldersFromIds(folderIds) };
}

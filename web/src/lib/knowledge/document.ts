import {
  extractMarkers,
  isKnowledgeKey,
  knowledgePreview,
  mentionSource,
  normalizeKey,
  replaceMentionKey,
} from "./markers";
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
  /** Raw status text when it is not one of the three legal values. */
  invalidStatus: string | null;
  summary: string;
  /** Mention ids in the body, first-seen order. */
  refs: string[];
  /** Body after the declaration fence. The fence itself is not included. */
  body: string;
  folderId: string | null;
  x: number | null;
  y: number | null;
  w: number | null;
  h: number | null;
  /** True when the fence contains a `summary` line. */
  hasSummary: boolean;
  /** True when the fence still has a legacy `ref :` line. */
  hadRefLine: boolean;
  /** Field lines this parser does not own, in source order. */
  extras: string[];
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

function citedIds(body: string): string[] {
  return extractMarkers(body);
}

/** `[[node : key]]` from files written before Mention shortcodes. */
const LEGACY_MARKER = /\[\[\s*node\s*:\s*([^\]\r\n]+?)\s*\]\]/g;

function rewriteLegacyMentions(body: string): string {
  return body.replace(LEGACY_MARKER, (full, raw: string) => {
    const key = normalizeKey(raw);
    if (!isKnowledgeKey(key)) return full;
    return mentionSource(key);
  });
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
  let invalidStatus: string | null = null;
  let summary = "";
  let body = source;
  let hasSummary = false;
  let hadRefLine = false;
  let x: number | null = null;
  let y: number | null = null;
  let w: number | null = null;
  let h: number | null = null;
  const extras: string[] = [];
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
        if (!field) {
          extras.push(trimmed);
          continue;
        }
        const name = field[1] ?? "";
        const value = (field[2] ?? "").trim();
        if (name === "node" && !sawNode) {
          sawNode = true;
          key = value;
        } else if (name === "status") {
          const parsed = parseStatus(value);
          if (parsed) {
            status = parsed;
            invalidStatus = null;
          } else {
            invalidStatus = value;
          }
        } else if (name === "summary") {
          hasSummary = true;
          summary = value;
        } else if (name === "ref") {
          hadRefLine = true;
        } else if (name === "x") x = parseCoord(value);
        else if (name === "y") y = parseCoord(value);
        else if (name === "w") w = parseCoord(value);
        else if (name === "h") h = parseCoord(value);
        else if (name !== "ref") extras.push(trimmed);
      }
    }
  }
  return {
    path: normalizedPath,
    key,
    status,
    invalidStatus,
    summary,
    refs: citedIds(body),
    body,
    folderId: folderIdOf(normalizedPath),
    x,
    y,
    w,
    h,
    hasSummary,
    hadRefLine,
    extras,
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
  invalidStatus?: string | null;
  body: string;
  summary?: string;
  x?: number | null;
  y?: number | null;
  w?: number | null;
  h?: number | null;
  extras?: string[];
}): string {
  const body = doc.body.replace(/\s+$/, "");
  const lines = [
    "```node",
    `node : ${doc.key}`,
    `status : ${doc.invalidStatus != null ? doc.invalidStatus : doc.status}`,
    `summary : ${doc.summary ?? ""}`,
  ];
  for (const line of [
    coordLine("x", doc.x),
    coordLine("y", doc.y),
    coordLine("w", doc.w),
    coordLine("h", doc.h),
  ]) {
    if (line) lines.push(line);
  }
  for (const extra of doc.extras ?? []) {
    const trimmed = extra.trim();
    if (trimmed) lines.push(trimmed);
  }
  lines.push("```", "", body, "");
  return lines.join("\n");
}

/**
 * Rewrite a legacy file once. `[[node : key]]` becomes a Mention shortcode,
 * `ref :` lines are dropped, and a missing summary is filled from the body.
 * A file that already uses shortcodes and has a summary is left unchanged.
 */
export function upgradeKnowledgeMarkdown(markdown: string): string | null {
  const parsed = parseKnowledgeMarkdown("node.md", markdown);
  if (!parsed.key) return null;
  const body = rewriteLegacyMentions(parsed.body);
  if (body === parsed.body && !parsed.hadRefLine && parsed.hasSummary) return null;
  return renderKnowledgeMarkdown({
    key: normalizeKey(parsed.key),
    status: parsed.status,
    invalidStatus: parsed.invalidStatus,
    summary: parsed.summary || knowledgePreview(body, 1),
    body,
    x: parsed.x,
    y: parsed.y,
    w: parsed.w,
    h: parsed.h,
    extras: parsed.extras,
  });
}

/** Rewrite mention ids. Declaration lines are left alone. */
export function replaceKnowledgeKey(text: string, from: string, to: string): string {
  return replaceMentionKey(text, from, to);
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
      invalidStatus: file.invalidStatus,
      folderId: file.folderId,
      path: file.path,
      x: file.x,
      y: file.y,
      w: file.w,
      h: file.h,
      extras: file.extras,
    };
  });

  const folderIds = new Set<string>();
  for (const dir of directories) addFolderChain(folderIds, dir);
  for (const node of nodes) {
    if (node.folderId) addFolderChain(folderIds, node.folderId);
  }
  return { nodes, folders: foldersFromIds(folderIds) };
}

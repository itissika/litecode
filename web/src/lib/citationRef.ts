import { defaultUrlTransform } from "react-markdown";

import { isWorkspaceFileRef } from "./knowledge/markers";

/** Workspace file, line, or symbol named by a `file:` markdown link. */
export type CitationTarget =
  | { kind: "file"; path: string }
  | { kind: "line"; path: string; line: number }
  | { kind: "symbol"; path: string; symbol: string };

const FILE_SCHEME = /^file:/i;
const SYMBOL = /^[A-Za-z0-9_.$]+$/;
const LINE = /^L(\d+)$/;

/**
 * Keep `file:` destinations. react-markdown's default allowlist drops them,
 * which would erase the citation before the anchor renderer runs.
 */
export function citationUrlTransform(value: string): string {
  if (FILE_SCHEME.test(value)) return value;
  return defaultUrlTransform(value);
}

/**
 * Parse a closed `file:` link. Illegal paths (empty, `..`, drive letters)
 * return null so the caller never queries them.
 */
export function parseFileCitation(href: string): CitationTarget | null {
  if (!FILE_SCHEME.test(href)) return null;
  const raw = href.slice(5);
  const hashAt = raw.indexOf("#");
  const beforeHash = hashAt >= 0 ? raw.slice(0, hashAt) : raw;
  const queryAt = beforeHash.indexOf("?");
  const pathPart = queryAt >= 0 ? beforeHash.slice(0, queryAt) : beforeHash;
  const path = normalizeRelative(pathPart);
  if (!path) return null;

  const fragment = hashAt >= 0 ? raw.slice(hashAt + 1) : "";
  const located = readFragment(fragment);
  if (located.line != null) return { kind: "line", path, line: located.line };
  if (located.symbol) return { kind: "symbol", path, symbol: located.symbol };
  return { kind: "file", path };
}

export function citationCacheKey(workspace: string, target: CitationTarget): string {
  const line = target.kind === "line" ? String(target.line) : "";
  const symbol = target.kind === "symbol" ? target.symbol : "";
  return `${workspace}\0${target.path}\0${line}\0${symbol}`;
}

function normalizeRelative(raw: string): string | null {
  let decoded: string;
  try {
    decoded = decodeURIComponent(raw);
  } catch {
    return null;
  }
  const slash = decoded.replace(/\\/g, "/").replace(/^\/+/, "");
  if (!slash || /^[a-zA-Z]:/.test(slash)) return null;
  const parts: string[] = [];
  for (const seg of slash.split("/")) {
    if (seg === "" || seg === ".") continue;
    if (seg === "..") return null;
    parts.push(seg);
  }
  if (parts.length === 0) return null;
  return parts.join("/");
}

function readFragment(fragment: string): { line?: number; symbol?: string } {
  if (!fragment) return {};
  let frag: string;
  try {
    frag = decodeURIComponent(fragment);
  } catch {
    return {};
  }
  const lineMatch = LINE.exec(frag);
  if (lineMatch) {
    const n = Number(lineMatch[1]);
    if (Number.isInteger(n) && n >= 1) return { line: n };
    return {};
  }
  if (SYMBOL.test(frag)) return { symbol: frag };
  return {};
}

/** Where a file-ish markdown link in assistant prose goes. */
export type WorkspaceLink =
  | { action: "chip"; path: string; line: number | null; symbol: string | null }
  | { action: "text" }
  | { action: "link" };

const HAS_SCHEME = /^[a-zA-Z][a-zA-Z0-9+.-]*:/;
const HAS_EXTENSION = /\.[A-Za-z0-9_-]+$/;

/**
 * Classify a markdown link in assistant prose. `file:...` and a relative path
 * that names a workspace file become citation chips, so the link opens in the
 * editor. A file-ish target we cannot open becomes text: it never looks
 * clickable and never opens a browser. Anything else — a web link, an anchor,
 * a bare word — stays an ordinary link.
 */
export function classifyWorkspaceLink(href: string | undefined): WorkspaceLink {
  if (!href) return { action: "link" };
  if (FILE_SCHEME.test(href)) {
    const target = parseFileCitation(href);
    if (!target) return { action: "text" };
    return {
      action: "chip",
      path: target.path,
      line: target.kind === "line" ? target.line : null,
      symbol: target.kind === "symbol" ? target.symbol : null,
    };
  }
  if (HAS_SCHEME.test(href) || href.startsWith("//") || href.startsWith("#")) {
    return { action: "link" };
  }

  const hashAt = href.indexOf("#");
  const beforeHash = hashAt >= 0 ? href.slice(0, hashAt) : href;
  const queryAt = beforeHash.indexOf("?");
  const rawPath = queryAt >= 0 ? beforeHash.slice(0, queryAt) : beforeHash;
  const fragment = hashAt >= 0 ? href.slice(hashAt + 1) : "";
  const { line = null, symbol = null } = readFragment(fragment);

  let path: string;
  try {
    path = decodeURIComponent(rawPath).replace(/\\/g, "/").replace(/^\.\//, "");
  } catch {
    return { action: "text" };
  }
  if (path.endsWith("/")) path = path.replace(/\/+$/, "");

  const pathLike =
    rawPath.includes("/") ||
    rawPath.endsWith("/") ||
    HAS_EXTENSION.test(path) ||
    line !== null;
  if (!pathLike) return { action: "link" };
  if (!isWorkspaceFileRef(path)) return { action: "text" };
  return { action: "chip", path, line, symbol };
}

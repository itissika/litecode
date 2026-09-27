import { defaultUrlTransform } from "react-markdown";

/** Workspace file, line, or symbol named by a `file:` markdown link. */
export type CitationTarget =
  | { kind: "file"; path: string }
  | { kind: "line"; path: string; line: number }
  | { kind: "symbol"; path: string; symbol: string };

const FILE_SCHEME = /^file:/i;
const HTTP_SCHEME = /^https?:\/\//i;
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

export function isHttpCitation(href: string): boolean {
  return HTTP_SCHEME.test(href);
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

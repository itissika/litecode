import { defaultUrlTransform } from "react-markdown";

import {
  isWorkspaceFileRef,
  normalizeRefPath,
  parseLineAnchor,
  parseLineSpan,
  parseWorkspaceDest,
} from "./knowledge/markers";

/** Workspace file, line, or symbol named by a `file:` markdown link. */
export type CitationTarget =
  | { kind: "file"; path: string }
  | { kind: "line"; path: string; line: number }
  | { kind: "symbol"; path: string; symbol: string };

const FILE_SCHEME = /^file:/i;
const SYMBOL = /^[A-Za-z0-9_.$]+$/;
const LINE = /^L(\d+)$/;

/**
 * Keep `file:` destinations and relative workspace links intact.
 * react-markdown's default allowlist drops `file:`, which would erase citations
 * before the anchor renderer runs.
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

/** Where a markdown link in prose goes. */
export type WorkspaceLink =
  | {
      action: "chip";
      path: string;
      line: number | null;
      lines: string | null;
      badLines: string | null;
      symbol: string | null;
    }
  | { action: "text" }
  | { action: "link" };

const HAS_SCHEME = /^[a-zA-Z][a-zA-Z0-9+.-]+:/;
const HAS_EXTENSION = /\.[A-Za-z0-9_-]+$/;

/**
 * Classify a Markdown link in prose or knowledge documents.
 *
 * - External URLs (`https://...`, `mailto:...`, `//...`) and `#anchors` stay links.
 * - Workspace citations (`[a.rs](src/a.rs)`, `[a.rs:12-30](src/a.rs#L12-L30)`,
 *   `[Store.save](src/a.rs#Store.save)`, `[seq](knowledge/contracts/seq.md)`)
 *   become interactive chips.
 * - When `knowledgeBody` is true (inside a knowledge node), even illegal paths
 *   or bare directory names (like `[src](src)` or `[bad](../x)`) still render as
 *   chips so the backend's authoritative verdict displays on them.
 */
export function classifyWorkspaceLink(
  href: string | undefined,
  options?: { knowledgeBody?: boolean },
): WorkspaceLink {
  if (!href) return { action: "link" };
  if (FILE_SCHEME.test(href)) {
    const parsed = parseWorkspaceDest(href);
    if (!parsed) return { action: "text" };
    const cleanPath = normalizeRefPath(parsed.path).replace(/^\/+/, "");
    if (!isWorkspaceFileRef(cleanPath)) {
      return options?.knowledgeBody
        ? {
            action: "chip",
            path: normalizeRefPath(parsed.path),
            line: null,
            lines: parsed.lines,
            badLines: parsed.badLines,
            symbol: parsed.symbol,
          }
        : { action: "text" };
    }
    const span = parsed.lines ? (parseLineSpan(parsed.lines) ?? parseLineAnchor(parsed.lines)) : null;
    return {
      action: "chip",
      path: cleanPath,
      line: span?.start ?? null,
      lines: parsed.lines,
      badLines: parsed.badLines,
      symbol: parsed.symbol,
    };
  }
  if (HAS_SCHEME.test(href) || href.startsWith("//") || href.startsWith("#")) {
    return { action: "link" };
  }

  const parsed = parseWorkspaceDest(href);
  if (!parsed) return { action: "link" };

  const rawPath = parsed.path;
  const normalized = normalizeRefPath(rawPath);
  const stripped = options?.knowledgeBody
    ? normalized
    : normalized.replace(/^\.\//, "").replace(/\/+$/, "");

  const span = parsed.lines ? parseLineSpan(parsed.lines) : null;
  if (options?.knowledgeBody) {
    return {
      action: "chip",
      path: normalized,
      line: span?.start ?? null,
      lines: parsed.lines,
      badLines: parsed.badLines,
      symbol: parsed.symbol,
    };
  }

  const pathLike =
    rawPath.includes("/") ||
    rawPath.includes("\\") ||
    rawPath.endsWith("/") ||
    HAS_EXTENSION.test(stripped) ||
    span !== null ||
    parsed.symbol !== null;
  if (!pathLike) return { action: "link" };
  if (!isWorkspaceFileRef(stripped)) return { action: "text" };
  return {
    action: "chip",
    path: stripped,
    line: span?.start ?? null,
    lines: parsed.lines,
    badLines: parsed.badLines,
    symbol: parsed.symbol,
  };
}

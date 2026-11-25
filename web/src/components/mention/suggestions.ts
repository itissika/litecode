import {
  fetchMentionPaths,
  fetchSymbols,
  type WorkspaceSymbol,
} from "../../api/workspace";
import type { KnowledgeMentionCandidate } from "../../lib/knowledge/flowProjection";
import { fileLabel, normalizeRefPath } from "../../lib/knowledge/markers";

export interface LockedFile {
  path: string;
  name: string;
}

type MentionPhase =
  | { mode: "unified"; filter: string }
  | { mode: "symbol"; filter: string; locked: LockedFile };

export interface FileCandidate {
  path: string;
  file: boolean;
}

export interface SymbolCandidate {
  path: string;
  chain: string;
  summary: string;
  start: number;
  end: number;
}

/** `@` query after the trigger. `#` locks the highlighted file and filters its symbols. */
export function phaseForQuery(query: string, locked: LockedFile | null): MentionPhase {
  if (locked && query.startsWith(`${locked.name}#`)) {
    return {
      mode: "symbol",
      filter: query.slice(locked.name.length + 1),
      locked,
    };
  }
  return { mode: "unified", filter: query };
}

/** Text written into the editor when `#` locks a file. The full path stays in plugin state. */
export function lockedQuery(name: string): string {
  return `@${name}#`;
}

export function filterSymbols(symbols: readonly WorkspaceSymbol[], filter: string): WorkspaceSymbol[] {
  const q = filter.trim().toLowerCase();
  if (!q) return symbols.slice(0, 12);
  return symbols
    .filter(
      (item) => item.chain.toLowerCase().includes(q) || item.name.toLowerCase().includes(q),
    )
    .slice(0, 12);
}

export function symbolCandidate(path: string, item: WorkspaceSymbol): SymbolCandidate {
  return {
    path,
    chain: item.chain,
    summary: item.summary,
    start: item.start_line,
    end: item.end_line,
  };
}

let fileRequest = 0;

export async function fileCandidates(
  query: string,
  signal?: AbortSignal,
): Promise<FileCandidate[]> {
  const cleaned = query
    .trim()
    .replace(/\\/g, "/")
    .replace(/^\/+/, "")
    .replace(/[*?[\]]/g, "");
  if (!cleaned || cleaned.includes("..")) return [];
  const request = ++fileRequest;
  try {
    const entries = await fetchMentionPaths(cleaned, signal);
    if (request !== fileRequest || signal?.aborted) return [];
    return entries.slice(0, 8).map((entry) => ({
      path: entry.path.replaceAll("\\", "/"),
      file: entry.file,
    }));
  } catch {
    return [];
  }
}

const symbolCache = new Map<string, WorkspaceSymbol[]>();

export function clearSymbolCache(): void {
  symbolCache.clear();
}

export async function symbolsFor(path: string): Promise<WorkspaceSymbol[]> {
  const cached = symbolCache.get(path);
  if (cached) return cached;
  const items = await fetchSymbols(path);
  symbolCache.set(path, items);
  return items;
}

/** Match valid knowledge node candidates by key prefix, key substring, or path substring. */
export function knowledgeItems(
  candidates: readonly KnowledgeMentionCandidate[],
  query: string,
  limit = 6,
): KnowledgeMentionCandidate[] {
  const q = query.trim().toLowerCase();
  if (!q) return candidates.slice(0, limit);
  const prefix: KnowledgeMentionCandidate[] = [];
  const contains: KnowledgeMentionCandidate[] = [];
  for (const item of candidates) {
    const keyLower = item.key.toLowerCase();
    if (keyLower.startsWith(q)) {
      prefix.push(item);
    } else if (
      keyLower.includes(q) ||
      item.path.toLowerCase().includes(q)
    ) {
      contains.push(item);
    }
  }
  return [...prefix, ...contains].slice(0, limit);
}

/** Exclude workspace files that are already represented as valid knowledge nodes. */
export function dedupeFileCandidates(
  files: readonly FileCandidate[],
  knowledge: readonly KnowledgeMentionCandidate[],
): FileCandidate[] {
  if (knowledge.length === 0) return [...files];
  const hidden = new Set<string>();
  for (const item of knowledge) {
    hidden.add(normalizeRefPath(item.path));
    hidden.add(normalizeRefPath(item.diskPath));
  }
  return files.filter((entry) => !hidden.has(normalizeRefPath(entry.path)));
}

export function fileDisplayName(path: string): string {
  return fileLabel(path);
}

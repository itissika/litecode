import {
  fetchMentionPaths,
  fetchSymbols,
  type WorkspaceSymbol,
} from "../../api/workspace";
import { fileLabel } from "../../lib/knowledge/markers";

export interface LockedFile {
  path: string;
  name: string;
}

export type FilePhase =
  | { mode: "file"; filter: string }
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

/** `/` query after the trigger. `#` locks the highlighted file and filters its symbols. */
export function phaseForQuery(query: string, locked: LockedFile | null): FilePhase {
  if (locked && query.startsWith(`${locked.name}#`)) {
    return {
      mode: "symbol",
      filter: query.slice(locked.name.length + 1),
      locked,
    };
  }
  return { mode: "file", filter: query };
}

/** Text written into the editor when `#` locks a file. The full path stays in plugin state. */
export function lockedQuery(name: string): string {
  return `/${name}#`;
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

export function mentionItems(candidates: readonly string[], query: string, limit = 8): string[] {
  const q = query.trim();
  const matched = q ? candidates.filter((key) => key.startsWith(q)) : candidates;
  return matched.slice(0, limit);
}

export function fileDisplayName(path: string): string {
  return fileLabel(path);
}

import { resolveCitations, type CitationRefHit } from "../api/workspace";
import { citationCacheKey, type CitationTarget } from "./citationRef";

/** Positive hits stay cached. Misses do not, so a later retry can see a new file. */
export interface CitationHit {
  exists: true;
  path: string;
  line?: number;
}

export type CitationLookup = CitationHit | { exists: false } | null;

const CHUNK = 32;

interface Queued {
  key: string;
  workspace: string;
  target: CitationTarget;
  promise: Promise<CitationLookup>;
  resolve: (hit: CitationLookup) => void;
}

const hits = new Map<string, CitationHit>();
const inflight = new Map<string, Promise<CitationLookup>>();
let queue: Queued[] = [];
let flushScheduled = false;

export function peekCitation(
  workspace: string,
  target: CitationTarget,
): CitationHit | undefined {
  return hits.get(citationCacheKey(workspace, target));
}

export function requestCitation(
  workspace: string,
  target: CitationTarget,
  options?: { fresh?: boolean },
): Promise<CitationLookup> {
  if (!workspace) return Promise.resolve(null);
  const key = citationCacheKey(workspace, target);
  const cached = hits.get(key);
  if (cached) return Promise.resolve(cached);
  if (!options?.fresh) {
    const pending = inflight.get(key);
    if (pending) return pending;
  }

  let resolve!: (hit: CitationLookup) => void;
  const promise = new Promise<CitationLookup>((done) => {
    resolve = done;
  });
  inflight.set(key, promise);
  queue.push({ key, workspace, target, promise, resolve });
  scheduleFlush();
  return promise;
}

export function resetCitationCacheForTests(): void {
  for (const item of queue) item.resolve(null);
  queue = [];
  flushScheduled = false;
  hits.clear();
  inflight.clear();
}

function scheduleFlush(): void {
  if (flushScheduled) return;
  flushScheduled = true;
  queueMicrotask(() => {
    flushScheduled = false;
    const batch = queue;
    queue = [];
    void runFlush(batch);
  });
}

async function runFlush(batch: Queued[]): Promise<void> {
  const byWorkspace = new Map<string, Queued[]>();
  for (const item of batch) {
    const list = byWorkspace.get(item.workspace) ?? [];
    list.push(item);
    byWorkspace.set(item.workspace, list);
  }
  await Promise.all(
    [...byWorkspace.values()].map((items) => flushWorkspace(items)),
  );
}

async function flushWorkspace(items: Queued[]): Promise<void> {
  const leaders: Queued[] = [];
  const followers = new Map<string, Queued[]>();
  for (const item of items) {
    const group = followers.get(item.key);
    if (!group) {
      followers.set(item.key, []);
      leaders.push(item);
    } else {
      group.push(item);
    }
  }

  for (let offset = 0; offset < leaders.length; offset += CHUNK) {
    const chunk = leaders.slice(offset, offset + CHUNK);
    let result: CitationRefHit[] | null = null;
    try {
      result = await resolveCitations(
        chunk.map((item) => {
          const body: { path: string; line?: number; symbol?: string } = {
            path: item.target.path,
          };
          if (item.target.kind === "line") body.line = item.target.line;
          if (item.target.kind === "symbol") body.symbol = item.target.symbol;
          return body;
        }),
      );
    } catch {
      result = null;
    }
    chunk.forEach((item, index) => {
      const hit = result ? normalizeHit(result[index]) : null;
      if (hit?.exists) hits.set(item.key, hit);
      settle(item, hit);
      for (const extra of followers.get(item.key) ?? []) settle(extra, hit);
    });
  }
}

function normalizeHit(hit: CitationRefHit | undefined): CitationLookup {
  if (!hit?.exists || !hit.path) return { exists: false };
  const line =
    typeof hit.line === "number" && Number.isInteger(hit.line) && hit.line >= 1
      ? hit.line
      : undefined;
  return { exists: true, path: hit.path, line };
}

function settle(item: Queued, hit: CitationLookup): void {
  if (inflight.get(item.key) === item.promise) inflight.delete(item.key);
  item.resolve(hit);
}

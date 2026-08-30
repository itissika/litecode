import type { EngineStatus, EngineWarmupState } from "./settings";
import { apiFetch } from "./auth";

export type TreeEntryKind = "file" | "dir";

export interface TreeEntry {
  name: string;
  path: string;
  kind: TreeEntryKind;
  size?: number;
}

export type WorkspaceChangeKind = "modified" | "created" | "deleted" | "renamed";

interface ApiOk<T> {
  ok: true;
  data: T;
}

interface ApiErr {
  ok: false;
  error: string;
}

type ApiResult<T> = ApiOk<T> | ApiErr;

async function parseJson<T>(res: Response): Promise<T> {
  const body = (await res.json()) as ApiResult<T>;
  if (!body.ok) {
    throw new Error(body.error || `HTTP ${res.status}`);
  }
  return body.data;
}

export async function fetchTree(
  path = "",
  depth = 1,
): Promise<TreeEntry[]> {
  const params = new URLSearchParams();
  if (path) params.set("path", path);
  params.set("depth", String(depth));

  const res = await apiFetch(`/api/workspace/tree?${params}`);
  if (!res.ok) {
    const text = await res.text();
    throw new Error(text || `HTTP ${res.status}`);
  }
  const data = await parseJson<{ entries: TreeEntry[] }>(res);
  return data.entries;
}

/** One-shot ancestor listing so a file becomes visible without N expand calls. */
export async function fetchTreeReveal(
  path: string,
): Promise<Record<string, TreeEntry[]>> {
  const params = new URLSearchParams();
  params.set("path", path);
  params.set("reveal", "1");
  const res = await apiFetch(`/api/workspace/tree?${params}`);
  if (!res.ok) {
    const text = await res.text();
    throw new Error(text || `HTTP ${res.status}`);
  }
  const data = await parseJson<{ by_dir: Record<string, TreeEntry[]> }>(res);
  return data.by_dir;
}

export interface GlobListing {
  entries: TreeEntry[];
  truncated: boolean;
}

export async function fetchGlob(pattern: string): Promise<GlobListing> {
  const params = new URLSearchParams();
  params.set("pattern", pattern);
  const res = await apiFetch(`/api/workspace/glob?${params}`);
  return parseJson<GlobListing>(res);
}

export async function readFile(path: string): Promise<string> {
  const params = new URLSearchParams({ path });
  const res = await apiFetch(`/api/workspace/file?${params}`);
  if (!res.ok) {
    const text = await res.text();
    throw new Error(text || `HTTP ${res.status}`);
  }
  const data = await parseJson<{ path: string; content: string }>(res);
  return data.content;
}

export async function writeFile(path: string, content: string): Promise<void> {
  const params = new URLSearchParams({ path });
  const res = await apiFetch(`/api/workspace/file?${params}`, {
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ content }),
  });
  if (!res.ok) {
    const text = await res.text();
    throw new Error(text || `HTTP ${res.status}`);
  }
  await parseJson<{ path: string }>(res);
}

export async function createFile(
  path: string,
  content: string,
): Promise<void> {
  const res = await apiFetch("/api/workspace/file", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ path, content }),
  });
  if (!res.ok) {
    const text = await res.text();
    throw new Error(text || `HTTP ${res.status}`);
  }
  await parseJson<{ path: string }>(res);
}

export async function deletePath(
  path: string,
  recursive = false,
): Promise<void> {
  const params = new URLSearchParams({ path });
  if (recursive) params.set("recursive", "true");

  const res = await apiFetch(`/api/workspace/file?${params}`, {
    method: "DELETE",
  });
  if (!res.ok) {
    const text = await res.text();
    throw new Error(text || `HTTP ${res.status}`);
  }
  await parseJson<{ path: string }>(res);
}

export async function mkdir(path: string): Promise<string> {
  const res = await apiFetch("/api/workspace/mkdir", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ path }),
  });
  if (!res.ok) {
    const text = await res.text();
    throw new Error(text || `HTTP ${res.status}`);
  }
  const data = await parseJson<{ path: string }>(res);
  return data.path;
}

export async function renamePath(
  from: string,
  to: string,
  overwrite = false,
): Promise<{ from: string; to: string }> {
  const res = await apiFetch("/api/workspace/rename", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ from, to, overwrite }),
  });
  if (!res.ok) {
    const text = await res.text();
    throw new Error(text || `HTTP ${res.status}`);
  }
  return parseJson<{ from: string; to: string }>(res);
}

export async function copyPath(
  from: string,
  to: string,
  overwrite = false,
): Promise<string> {
  const res = await apiFetch("/api/workspace/copy", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ from, to, overwrite }),
  });
  if (!res.ok) {
    const text = await res.text();
    throw new Error(text || `HTTP ${res.status}`);
  }
  const data = await parseJson<{ path: string }>(res);
  return data.path;
}

export async function writeBlob(
  path: string,
  bytes: Uint8Array,
  overwrite = false,
): Promise<string> {
  const params = new URLSearchParams({ path });
  const res = await apiFetch(`/api/workspace/blob?${params}`, {
    method: overwrite ? "PUT" : "POST",
    headers: { "Content-Type": "application/octet-stream" },
    body: bytes as unknown as BodyInit,
  });
  if (!res.ok) {
    const text = await res.text();
    throw new Error(text || `HTTP ${res.status}`);
  }
  const data = await parseJson<{ path: string }>(res);
  return data.path;
}

export type LspDepStatus = "available" | "missing" | "broken";

export interface LspServerProbe {
  id: string;
  command: string;
  sources: string[];
  status: LspDepStatus;
  install_hint?: string | null;
  size_hint?: string | null;
  installed_version?: string | null;
  error?: string | null;
  managed_path?: string | null;
  official_url?: string | null;
}

export interface LspInitFailure {
  id: string;
  error: string;
}

export async function probeLspServers(): Promise<LspServerProbe[]> {
  const res = await apiFetch("/api/workspace/lsp/probe");
  if (!res.ok) {
    const text = await res.text();
    throw new Error(text || `HTTP ${res.status}`);
  }
  const data = await parseJson<{ servers: LspServerProbe[] }>(res);
  return data.servers;
}

export async function initLspServers(
  servers: string[],
): Promise<{ servers: string[] }> {
  const res = await apiFetch("/api/workspace/lsp/init", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ servers }),
  });
  const body = await res.json();
  if (!res.ok) {
    const failed = (body as { failed?: LspInitFailure[] }).failed;
    if (failed?.length) {
      const detail = failed.map((f) => `${f.id}: ${f.error}`).join("; ");
      throw new Error(detail);
    }
    throw new Error(
      (body as ApiErr).error || `HTTP ${res.status}`,
    );
  }
  const data = body as ApiOk<{ servers: string[] }>;
  if (!data.ok || !data.data) {
    throw new Error("invalid LSP init response");
  }
  return data.data;
}

async function postEngineAction(
  path: string,
): Promise<{ desired: boolean }> {
  const res = await apiFetch(path, { method: "POST" });
  return parseJson<{ desired: boolean }>(res);
}

export function stopLsp(): Promise<{ desired: boolean }> {
  return postEngineAction("/api/workspace/lsp/stop");
}

/** Clear enabled server list and stop. Unlike stopLsp, does not keep servers. */
export function clearLspServers(): Promise<{ desired: boolean }> {
  return postEngineAction("/api/workspace/lsp/clear");
}

export function initRetrieval(): Promise<{ desired: boolean }> {
  return postEngineAction("/api/workspace/retrieval/init");
}

export function stopRetrieval(): Promise<{ desired: boolean }> {
  return postEngineAction("/api/workspace/retrieval/stop");
}

export type IndexStatus =
  | "absent"
  | "ready"
  | "stale"
  | "needs_rebuild"
  | "building"
  | "refreshing"
  | "failed";

export type IndexPhase = "scanning" | "embedding" | "saving" | "syncing";

export interface IndexingProgress {
  phase: IndexPhase;
  files_done: number;
  files_total: number;
  chunks_done: number;
}

export type RefreshAcceptedMode = "starting" | "in_progress" | "rebuild" | "incremental";

export async function refreshRetrieval(): Promise<{ desired: boolean; mode: RefreshAcceptedMode }> {
  const res = await apiFetch("/api/workspace/retrieval/refresh", { method: "POST" });
  return parseJson<{ desired: boolean; mode: RefreshAcceptedMode }>(res);
}

export interface InstallTask {
  task_id: string;
  server_id: string;
  status: 'installing' | 'done' | 'failed';
  error?: string | null;
  progress?: {
    downloaded_bytes: number;
    total_bytes?: number | null;
  } | null;
}

export async function installServer(serverId: string): Promise<InstallTask> {
  const res = await apiFetch("/api/workspace/lsp/install", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ server_id: serverId }),
  });
  return parseJson<InstallTask>(res);
}

export async function getInstallStatus(taskId: string): Promise<InstallTask> {
  const res = await apiFetch(`/api/workspace/lsp/install/status?task_id=${encodeURIComponent(taskId)}`);
  return parseJson<InstallTask>(res);
}

export interface RetrievalEngineDetail {
  desired: boolean;
  state?: EngineWarmupState | null;
  error?: string | null;
  usable: "stopped" | "unavailable" | "warming" | "ready";
  model: {
    model_dir?: string;
    model_path?: string;
    tokenizer_path?: string;
    model_found: boolean;
    tokenizer_found: boolean;
    ready: boolean;
  };
  index: {
    status: IndexStatus;
    progress?: IndexingProgress | null;
    exists: boolean;
    needs_rebuild: boolean;
    vectors_ready: boolean;
    indexed_files: number;
    indexed_chunks: number;
    created_at?: string | null;
    model_id?: string | null;
    embedder_id?: string | null;
    pipeline_version?: number | null;
    pending_updates?: number;
  };
  policy: {
    /** Product-internal hard-skip dirs (e.g. `.litecode`). */
    product_internal_dirs: string[];
    /** Index preset exclude globs (files∪search + product dirs). */
    exclude_globs: string[];
    extensions: string[];
    max_file_bytes: number;
    binary_files: boolean;
    lockfiles: boolean;
    minified_files: boolean;
  };
}

export interface LspInstanceStatusView {
  command: string;
  project_root: string;
  state: "starting" | "running" | "restarting" | "failed" | "stopped";
  index_settled: boolean;
  last_error?: string | null;
  restart_count: number;
}

export interface LspEngineDetail {
  desired: boolean;
  state?: EngineWarmupState | null;
  error?: string | null;
  usable: "stopped" | "unavailable" | "warming" | "ready";
  configured_servers: string[];
  probes: LspServerProbe[];
  /** Live language-server instances (Hub); optional for older payloads. */
  servers?: LspInstanceStatusView[];
}

export interface EnginesDetail {
  retrieval: RetrievalEngineDetail;
  lsp: LspEngineDetail;
}

export async function retrievalSearch(body: {
  query: string;
  corpus?: "code" | "session";
  case_sensitive?: boolean;
  whole_word?: boolean;
  is_regex?: boolean;
  include?: string;
  exclude?: string;
  top_k?: number;
  project?: string;
  /** 0-based hit offset for session corpus pagination. */
  offset?: number;
  /** When false, code corpus skips semantic lane (text-only). Default true.
   *  Session corpus: when false, keeps lexical hits only. */
  include_semantic?: boolean;
}): Promise<RetrievalSearchResult> {
  const res = await apiFetch("/api/workspace/retrieval/search", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  return parseJson<RetrievalSearchResult>(res);
}

export interface RetrievalSearchHit {
  path: string;
  start_line: number;
  end_line: number;
  summary: string;
  score: number;
}

export interface SessionSearchHitRow {
  line: number;
  seq: number;
  summary: string;
}

export interface SessionSearchGroup {
  session_id: string;
  created_time: number;
  updated_time: number;
  path: string;
  match_count: number;
  hits: SessionSearchHitRow[];
}

export interface SessionSearchPage {
  groups: SessionSearchGroup[];
  offset: number;
  next_offset: number;
  has_more: boolean;
}

export interface RetrievalSearchResult {
  text: RetrievalSearchHit[];
  semantic?: RetrievalSearchHit[] | null;
  session_page?: SessionSearchPage | null;
}

export async function getEngines(): Promise<EnginesSnapshot> {
  const res = await apiFetch("/api/workspace/engines");
  return parseJson<EnginesSnapshot>(res);
}

export interface EnginesSnapshot {
  engines: Record<string, EngineStatus>;
  lsp_servers: LspInstanceStatusView[];
}

export async function getEnginesDetail(): Promise<EnginesDetail> {
  const res = await apiFetch("/api/workspace/engines/detail");
  return parseJson<EnginesDetail>(res);
}

export interface GitFile {
  path: string;
  status: string;
  orig_path?: string | null;
  untracked: boolean;
}

export interface GitStatus {
  is_repo: boolean;
  branch: string | null;
  upstream_ahead: number;
  upstream_behind: number;
  staged: GitFile[];
  changes: GitFile[];
}

export interface GitCommitInfo {
  sha: string;
  parents: string[];
  subject: string;
  author: string;
  date: string;
  body: string;
}

export interface GitLog {
  is_repo: boolean;
  commits: GitCommitInfo[];
}

async function gitJson<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await apiFetch(path, init);
  return parseJson<T>(res);
}

export async function gitStatus(): Promise<GitStatus> {
  return gitJson<GitStatus>("/api/workspace/git/status");
}

export async function gitLog(limit = 50): Promise<GitLog> {
  const params = new URLSearchParams({ limit: String(limit) });
  return gitJson<GitLog>(`/api/workspace/git/log?${params}`);
}

export async function gitStage(paths: string[]): Promise<void> {
  await gitJson("/api/workspace/git/stage", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ paths }),
  });
}

export async function gitUnstage(paths: string[]): Promise<void> {
  await gitJson("/api/workspace/git/unstage", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ paths }),
  });
}

export async function gitRestore(paths: string[]): Promise<void> {
  await gitJson("/api/workspace/git/restore", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ paths }),
  });
}

export async function gitCommit(message: string): Promise<void> {
  await gitJson("/api/workspace/git/commit", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ message }),
  });
}

export async function gitPull(): Promise<void> {
  await gitJson("/api/workspace/git/pull", { method: "POST" });
}

export async function gitPush(): Promise<void> {
  await gitJson("/api/workspace/git/push", { method: "POST" });
}

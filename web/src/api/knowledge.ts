import { apiFetch } from "./auth";

/** Wire shapes of `GET /api/workspace/knowledge` (`docs/knowledge-references.md` §10.1). */
type WireSeverity = "error" | "warning";

interface WireKnowledgeIssue {
  severity: WireSeverity;
  code: string;
  message: string;
  ref_key: string | null;
  args: Record<string, unknown>;
}

interface WireKnowledgeRef {
  ref_key: string;
  kind: "node" | "file" | "symbol" | "range";
  key: string | null;
  path: string | null;
  symbol: string | null;
  lines: string | null;
  raw: string;
  status: string;
  severity: WireSeverity | null;
  target_path: string | null;
  args: Record<string, unknown>;
  message: string;
}

export interface WireKnowledgeNode {
  path: string;
  /** Legacy id for the `knowledge` tool. The frontend derives its own. */
  id: string;
  key: string;
  status: "enabled" | "disabled" | "pending";
  invalid_status: string | null;
  summary: string;
  body: string;
  x: number | null;
  y: number | null;
  w: number | null;
  h: number | null;
  extras: string[];
  folder_id: string | null;
  relations: string[];
  severity: WireSeverity | null;
  issues: WireKnowledgeIssue[];
  refs: WireKnowledgeRef[];
}

export interface WireKnowledgeSnapshot {
  version: number;
  epoch: string;
  ready: boolean;
  root: string | null;
  visibility: "public" | "private";
  head: string | null;
  folders: { id: string; name: string; parent_id: string | null }[];
  unknown: { path: string; name: string; folder_id: string | null }[];
  nodes: WireKnowledgeNode[];
}

/** `knowledge/changed` params. `paths` is a hint only. */
export interface KnowledgeChanged {
  version: number;
  epoch: string;
  paths?: string[];
}

/** One request, the whole corpus with every verdict. Throws on HTTP / network failure. */
export async function fetchKnowledgeSnapshot(): Promise<WireKnowledgeSnapshot> {
  const res = await apiFetch("/api/workspace/knowledge");
  if (!res.ok) {
    const text = await res.text().catch(() => "");
    throw new Error(text || `HTTP ${res.status}`);
  }
  const body = (await res.json()) as
    | { ok: true; data: WireKnowledgeSnapshot }
    | { ok: false; error?: string };
  if (!body.ok) throw new Error(body.error || "knowledge snapshot failed");
  return body.data;
}

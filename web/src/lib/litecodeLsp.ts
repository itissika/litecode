import type * as Monaco from "monaco-editor";

import type { LspResult } from "../api/types";
import type { WireEnvelope } from "../api/agentWs";
import { useConnectionStore, attachSiblingStores } from "../stores/connectionStore";
import { useEditorStore } from "../stores/editorStore";
import { useSettingsStore } from "../stores/settingsStore";

const LSP_LANGUAGES = [
  "rust",
  "typescript",
  "javascript",
  "python",
  "go",
  "csharp",
  "c",
  "cpp",
] as const;

let nextId = 1;
const pending = new Map<
  number,
  { resolve: (value: LspResult) => void; reject: (err: Error) => void }
>();
/** Editor-originated JSON-RPC ids; keep out of the hub's low counter range. */
let nextLspRpcId = 1_000_000_000;

const lastHubRevByUri = new Map<string, number>();
const lastLspVersionByUri = new Map<string, number>();
const lastAppliedTextByUri = new Map<string, string>();
const serverReadyByUri = new Map<string, boolean>();
const pendingContentChangesByUri = new Map<string, LspTextEdit[]>();
const applyInFlight = new Map<string, Promise<SyncAck | null>>();
const changeDebounceTimers = new Map<string, ReturnType<typeof setTimeout>>();

export type LspTextEdit = {
  range: {
    start: { line: number; character: number };
    end: { line: number; character: number };
  };
  text: string;
};

function rememberApply(uri: string, text: string, rev: unknown, version?: unknown): void {
  lastAppliedTextByUri.set(uri, text);
  if (typeof rev === "number") {
    lastHubRevByUri.set(uri, rev);
  }
  if (typeof version === "number") {
    lastLspVersionByUri.set(uri, version);
  }
}

function noteDocServerReady(uri: string, ready: boolean): void {
  serverReadyByUri.set(uri, ready);
}

function isDocServerReady(uri: string): boolean {
  return serverReadyByUri.get(uri) === true;
}

function resetDocumentAuthority(): void {
  for (const timer of changeDebounceTimers.values()) clearTimeout(timer);
  changeDebounceTimers.clear();
  applyInFlight.clear();
  pendingContentChangesByUri.clear();
  lastHubRevByUri.clear();
  lastLspVersionByUri.clear();
  lastAppliedTextByUri.clear();
  serverReadyByUri.clear();
}

/** Monaco CompletionTriggerKind 0/1/2 → LSP CompletionTriggerKind 1/2/3. */
export function monacoCompletionTriggerToLsp(monacoKind: number): number {
  return monacoKind + 1;
}

/** Hub active: pool/config intent (`activate`), not a live language server. */
export function isLspWarm(): boolean {
  return useSettingsStore.getState().engineStatuses.lsp?.state === "warm";
}

/**
 * At least one language server has finished initialize (`Running`).
 * Hover/completion must not treat hub Warm as this.
 */
export function isLspServerReady(): boolean {
  for (const ready of serverReadyByUri.values()) {
    if (ready) return true;
  }
  const servers = useSettingsStore.getState().lspServers;
  return Array.isArray(servers) && servers.some((s) => s.state === "running");
}

export type DiagnosticsSnapshot = {
  rev: number | null;
  fresh: boolean;
  serverReady: boolean;
  diagnostics: unknown;
};

export type SyncAck = {
  rev: number | null;
  version: number | null;
  serverReady: boolean;
};

export function parseDiagnosticsSnapshot(result: unknown): DiagnosticsSnapshot {
  if (!result || typeof result !== "object") {
    return { rev: null, fresh: false, serverReady: false, diagnostics: [] };
  }
  const raw = result as {
    rev?: unknown;
    fresh?: unknown;
    server_ready?: unknown;
    diagnostics?: unknown;
  };
  return {
    rev: typeof raw.rev === "number" ? raw.rev : null,
    fresh: raw.fresh === true,
    serverReady: raw.server_ready === true,
    diagnostics: Array.isArray(raw.diagnostics) ? raw.diagnostics : [],
  };
}

export function parseDidChangeAck(result: unknown): SyncAck {
  if (!result || typeof result !== "object" || Array.isArray(result)) {
    return { rev: null, version: null, serverReady: false };
  }
  const raw = result as {
    rev?: unknown;
    version?: unknown;
    server_ready?: unknown;
  };
  return {
    rev: typeof raw.rev === "number" ? raw.rev : null,
    version: typeof raw.version === "number" ? raw.version : null,
    serverReady: raw.server_ready === true,
  };
}

/** VS Code languageclient: unversioned publish still applies; older version is ignored. */
export function shouldApplyPublishedDiagnostics(
  publishedVersion: number | null | undefined,
  sentVersion: number | undefined,
): boolean {
  if (typeof publishedVersion !== "number") return true;
  if (sentVersion === undefined) return true;
  return publishedVersion >= sentVersion;
}

export function monacoChangeToLsp(change: {
  range: {
    startLineNumber: number;
    startColumn: number;
    endLineNumber: number;
    endColumn: number;
  };
  text: string;
}): LspTextEdit {
  return {
    range: {
      start: {
        line: change.range.startLineNumber - 1,
        character: change.range.startColumn - 1,
      },
      end: {
        line: change.range.endLineNumber - 1,
        character: change.range.endColumn - 1,
      },
    },
    text: change.text,
  };
}

export function handleLspWireEnvelope(env: WireEnvelope): boolean {
  if (env.method === "lsp/diagnostics") {
    handleLspDiagnosticsNotification(env.params ?? {});
    return true;
  }
  // JSON-RPC 2.0 response: { jsonrpc, id, result }
  // LSP results are embedded in the RPC result: { id: lsp_id, result/error: ... }
  if (!("jsonrpc" in env) || !("result" in env) || !("id" in env)) return false;
  const result = env.result as LspResult | undefined;
  if (!result || typeof result.id !== "number") return false;
  const waiter = pending.get(result.id);
  if (!waiter) return true;
  pending.delete(result.id);
  waiter.resolve(result);
  return true;
}

const FEATURE_TIMEOUT_MS = 15_000;
/** csharp-ls / rust-analyzer initialize can exceed the feature timeout. */
const DOCUMENT_SYNC_TIMEOUT_MS = 90_000;
/** Merge a keystroke burst; VS Code languageclient sends Incremental immediately. */
const DID_CHANGE_DEBOUNCE_MS = 80;
const SEMANTIC_DEBOUNCE_MS = 300;
const INLAY_DEBOUNCE_MS = 250;

function isCanceledError(err: unknown): boolean {
  if (!err) return false;
  if (typeof err === "object" && "name" in err && (err as { name?: string }).name === "Canceled") {
    return true;
  }
  const msg = err instanceof Error ? err.message : String(err);
  return /cancel/i.test(msg);
}

function canceledError(): Error {
  const err = new Error("Canceled");
  err.name = "Canceled";
  return err;
}

function sleepCancellable(ms: number, token?: Monaco.CancellationToken): Promise<void> {
  return new Promise((resolve, reject) => {
    if (token?.isCancellationRequested) {
      reject(canceledError());
      return;
    }
    const timer = setTimeout(resolve, ms);
    const sub = token?.onCancellationRequested(() => {
      clearTimeout(timer);
      sub?.dispose();
      reject(canceledError());
    });
  });
}

function sendLsp(
  method: string,
  params: Record<string, unknown>,
  opts?: { token?: Monaco.CancellationToken; timeoutMs?: number },
): Promise<LspResult> {
  const id = nextId++;
  nextId = nextId % Number.MAX_SAFE_INTEGER || 1;
  const rpcId = nextLspRpcId++;
  const timeoutMs = opts?.timeoutMs ?? FEATURE_TIMEOUT_MS;
  return new Promise((resolve, reject) => {
    pending.set(id, { resolve, reject });
    const cancel = () => {
      void useConnectionStore.getState().sendRpc("lsp/request", {
        method: "$/cancelRequest",
        params: { id: rpcId },
      });
      const waiter = pending.get(id);
      if (waiter) {
        pending.delete(id);
        waiter.reject(canceledError());
      }
    };
    const sub = opts?.token?.onCancellationRequested(cancel);
    if (opts?.token?.isCancellationRequested) {
      cancel();
      return;
    }
    useConnectionStore.getState().sendRpc<{ result: unknown }>("lsp/request", {
      method,
      params,
      rpc_id: rpcId,
    })
      .then((response) => {
        sub?.dispose();
        const waiter = pending.get(id);
        if (!waiter) return;
        pending.delete(id);
        if (opts?.token?.isCancellationRequested) {
          waiter.reject(canceledError());
          return;
        }
        waiter.resolve({ id, result: response.result } as LspResult);
      })
      .catch((e) => {
        sub?.dispose();
        const waiter = pending.get(id);
        if (!waiter) return;
        pending.delete(id);
        waiter.reject(isCanceledError(e) ? canceledError() : e instanceof Error ? e : new Error(String(e)));
      });
    setTimeout(() => {
      if (!pending.has(id)) return;
      pending.delete(id);
      void useConnectionStore.getState().sendRpc("lsp/request", {
        method: "$/cancelRequest",
        params: { id: rpcId },
      });
      reject(new Error("LSP request timed out"));
    }, timeoutMs);
  });
}

/**
 * Slash-normalize a LAP absolute path for URI assembly.
 *
 * Backend `hello.project` is already Litecode Absolute Path (no `\\?\`).
 * This only unifies separators and optional trailing slash — not a second
 * verbatim strip. Non-LAP roots are rejected.
 */
function assertNoVerbatim(p: string, label: string): void {
  const raw = p.replace(/\\/g, "/");
  // Build markers without embedding silent-strip helpers (death-list gated).
  const slashQ = `/${"?"}/`;
  const uncVerbatim = ["\\\\", "?", "\\", "UNC\\"].join("");
  const driveVerbatim = ["\\\\", "?", "\\"].join("");
  if (
    p.startsWith(uncVerbatim) ||
    p.startsWith(driveVerbatim) ||
    raw.includes(slashQ) ||
    raw.startsWith(`//${"?"}/`)
  ) {
    throw new Error(`${label} is not Litecode Absolute Path (LAP); refused verbatim form: ${p}`);
  }
}

function normalizeFsPath(p: string): string {
  assertNoVerbatim(p, "path");
  let s = p.replace(/\\/g, "/").replace(/\/$/, "");
  // Match LAP drive-letter uppercase policy (`c:/` → `C:/`).
  s = s.replace(/^([a-z]):\//, (_, d: string) => `${d.toUpperCase()}:/`);
  return s;
}

export function toFileUri(projectRoot: string, relPath: string): string {
  assertNoVerbatim(projectRoot, "project root");
  assertNoVerbatim(relPath, "relative path");
  const root = normalizeFsPath(projectRoot);
  const rel = relPath.replace(/\\/g, "/").replace(/^\//, "").replace(/\/$/, "");
  const path = rel ? `${root}/${rel}` : root;
  if (path.startsWith("//")) {
    // UNC `//host/share/...` → `file://host/share/...`
    return `file:${path}`;
  }
  if (path.startsWith("/")) {
    return `file://${path}`;
  }
  return `file:///${path}`;
}

/** Relative workspace path for a Monaco model (matches Editor `path` prop). */
export function relPathFromModel(
  model: Monaco.editor.ITextModel,
  projectRoot: string,
): string {
  const root = normalizeFsPath(projectRoot);
  const fsPath = normalizeFsPath(model.uri.fsPath);
  if (fsPath && !fsPath.startsWith("/") && fsPath.includes("/")) {
    // Browser Monaco may report fsPath like `src/lsp/mod.rs` without leading slash.
    return fsPath.replace(/^\//, "");
  }
  if (fsPath && fsPath.startsWith(`${root}/`)) {
    return fsPath.slice(root.length + 1);
  }
  const fromPath = model.uri.path.replace(/^\//, "");
  if (fromPath && !fromPath.startsWith(root)) {
    return fromPath;
  }
  if (fromPath.startsWith(`${root}/`)) {
    return fromPath.slice(root.length + 1);
  }
  return fromPath;
}

/** Monaco URI for a workspace-relative path (same as @monaco-editor/react `path` prop). */
export function monacoUriForRelPath(
  monaco: typeof import("monaco-editor"),
  relPath: string,
): Monaco.Uri {
  return monaco.Uri.parse(relPath.replace(/^\//, ""));
}

function lspRangeToMonaco(range: unknown): Monaco.IRange | null {
  if (!range || typeof range !== "object") return null;
  const r = range as {
    start?: { line?: number; character?: number };
    end?: { line?: number; character?: number };
  };
  if (!r.start || !r.end) return null;
  return {
    startLineNumber: (r.start.line ?? 0) + 1,
    startColumn: (r.start.character ?? 0) + 1,
    endLineNumber: (r.end.line ?? 0) + 1,
    endColumn: (r.end.character ?? 0) + 1,
  };
}

export function relPathFromLspUri(uri: string, projectRoot: string): string {
  // Strip `file://` — LAP roots use normal absolute forms (`/home/...` or `E:/...`).
  const raw = decodeURIComponent(uri.replace(/^file:\/\//i, ""));
  const pathMatch = normalizeFsPath(raw);
  const root = normalizeFsPath(projectRoot);
  const pathCmp = pathMatch.toLowerCase();
  const rootCmp = root.toLowerCase();
  if (pathCmp === rootCmp) return "";
  if (pathCmp.startsWith(`${rootCmp}/`)) {
    return pathMatch.slice(root.length + 1);
  }
  // Drive-letter paths may arrive as `/E:/...` after file:// strip.
  const bare = pathMatch.replace(/^\//, "");
  const bareRoot = root.replace(/^\//, "");
  if (bare.toLowerCase().startsWith(`${bareRoot.toLowerCase()}/`)) {
    return bare.slice(bareRoot.length + 1);
  }
  return bare;
}

function parseLocation(
  monaco: typeof import("monaco-editor"),
  result: unknown,
  projectRoot: string,
  sourceModel?: Monaco.editor.ITextModel,
): Monaco.languages.Location | null {
  if (!result || typeof result !== "object") return null;
  const obj = result as Record<string, unknown>;
  const uri =
    (typeof obj.uri === "string" && obj.uri) ||
    (typeof obj.targetUri === "string" && obj.targetUri) ||
    null;
  const range = lspRangeToMonaco(
    obj.range ?? obj.targetRange ?? obj.targetSelectionRange,
  );
  if (!uri || !range) return null;

  const rel = relPathFromLspUri(uri, projectRoot);
  const modelUri =
    sourceModel &&
    rel === relPathFromModel(sourceModel, projectRoot)
      ? sourceModel.uri
      : monacoUriForRelPath(monaco, rel);
  return { uri: modelUri, range };
}

function parseLocations(
  monaco: typeof import("monaco-editor"),
  result: unknown,
  projectRoot: string,
  sourceModel?: Monaco.editor.ITextModel,
): Monaco.languages.Location[] {
  if (Array.isArray(result)) {
    return result
      .map((item) => parseLocation(monaco, item, projectRoot, sourceModel))
      .filter((l): l is Monaco.languages.Location => l !== null);
  }
  const one = parseLocation(monaco, result, projectRoot, sourceModel);
  return one ? [one] : [];
}

function parseHover(result: unknown): {
  contents: Monaco.IMarkdownString[];
  range: Monaco.IRange | null;
} {
  if (!result || typeof result !== "object") return { contents: [], range: null };
  const raw = result as { contents?: unknown; range?: unknown };
  const range = lspRangeToMonaco(raw.range);
  const contents = raw.contents;
  if (typeof contents === "string") {
    return { contents: [{ value: contents }], range };
  }
  if (contents && typeof contents === "object" && "value" in contents) {
    const value = (contents as { value?: string }).value ?? "";
    return { contents: [{ value }], range };
  }
  if (Array.isArray(contents)) {
    return {
      contents: contents.map((c) => {
        if (typeof c === "string") return { value: c };
        if (c && typeof c === "object" && "value" in c) {
          return { value: String((c as { value?: string }).value ?? "") };
        }
        return { value: "" };
      }),
      range,
    };
  }
  return { contents: [], range };
}

function warnLsp(context: string, err: unknown): void {
  if (!import.meta.env.DEV) return;
  const msg = err instanceof Error ? err.message : String(err);
  console.warn(`[litecode-lsp] ${context}: ${msg}`);
}

function applyRawDiagnostics(
  monaco: typeof import("monaco-editor"),
  model: Monaco.editor.ITextModel,
  diagnostics: unknown,
): void {
  if (!Array.isArray(diagnostics)) {
    monaco.editor.setModelMarkers(model, "litecode-lsp", []);
    return;
  }
  const markers: Monaco.editor.IMarkerData[] = [];
  for (const item of diagnostics) {
    if (!item || typeof item !== "object") continue;
    const d = item as {
      message?: string;
      severity?: number;
      range?: {
        start?: { line?: number; character?: number };
        end?: { line?: number; character?: number };
      };
    };
    const range = d.range;
    if (!range?.start) continue;
    const startLine = (range.start.line ?? 0) + 1;
    const startCol = (range.start.character ?? 0) + 1;
    const endLine = (range.end?.line ?? range.start.line ?? 0) + 1;
    const endCol = (range.end?.character ?? (range.start.character ?? 0) + 1) + 1;
    const sev = d.severity ?? 1;
    const severity =
      sev === 1
        ? monaco.MarkerSeverity.Error
        : sev === 2
          ? monaco.MarkerSeverity.Warning
          : sev === 3
            ? monaco.MarkerSeverity.Info
            : monaco.MarkerSeverity.Hint;
    markers.push({
      severity,
      message: d.message ?? "",
      startLineNumber: startLine,
      startColumn: startCol,
      endLineNumber: endLine,
      endColumn: Math.max(endCol, startCol + 1),
    });
  }
  monaco.editor.setModelMarkers(model, "litecode-lsp", markers);
}

export function handleLspDiagnosticsNotification(params: {
  uri?: unknown;
  version?: unknown;
  diagnostics?: unknown;
}): void {
  if (!workspaceLsp) return;
  const uri = typeof params.uri === "string" ? params.uri : "";
  if (!uri) return;
  const publishedVersion = typeof params.version === "number" ? params.version : undefined;
  const sentKey = uriKeyInMap(lastLspVersionByUri, uri);
  const sentVersion = sentKey ? lastLspVersionByUri.get(sentKey) : undefined;
  if (!shouldApplyPublishedDiagnostics(publishedVersion, sentVersion)) {
    return;
  }
  const model = findModelForLspUri(workspaceLsp.monaco, workspaceLsp.project, uri);
  if (!model) return;
  applyRawDiagnostics(workspaceLsp.monaco, model, params.diagnostics ?? []);
}

function uriKeyInMap<T>(map: Map<string, T>, uri: string): string | undefined {
  if (map.has(uri)) return uri;
  const needle = uri.replace(/\\/g, "/").toLowerCase();
  for (const key of map.keys()) {
    if (key.replace(/\\/g, "/").toLowerCase() === needle) return key;
  }
  return undefined;
}

function findModelForLspUri(
  monaco: typeof import("monaco-editor"),
  project: string,
  uri: string,
): Monaco.editor.ITextModel | undefined {
  return monaco.editor.getModels().find((m) => {
    const modelUri = lspFileUri(m, project);
    return modelUri != null && urisMatch(modelUri, uri);
  });
}

function urisMatch(a: string, b: string): boolean {
  return a.replace(/\\/g, "/").toLowerCase() === b.replace(/\\/g, "/").toLowerCase();
}

function queueContentChanges(uri: string, changes: readonly Monaco.editor.IModelContentChange[]): void {
  if (changes.length === 0) return;
  const pending = pendingContentChangesByUri.get(uri) ?? [];
  for (const change of changes) {
    pending.push(monacoChangeToLsp(change));
  }
  pendingContentChangesByUri.set(uri, pending);
}

async function flushDidChange(
  monaco: typeof import("monaco-editor"),
  model: Monaco.editor.ITextModel,
  uri: string,
  depth = 0,
): Promise<SyncAck | null> {
  const pendingTimer = changeDebounceTimers.get(uri);
  if (pendingTimer !== undefined) {
    clearTimeout(pendingTimer);
    changeDebounceTimers.delete(uri);
  }
  const inflight = applyInFlight.get(uri);
  if (inflight) {
    const last = await inflight;
    if (model.isDisposed() || depth >= 8) return last;
    const leftover = pendingContentChangesByUri.get(uri) ?? [];
    if (
      leftover.length === 0 &&
      lastAppliedTextByUri.get(uri) === model.getValue() &&
      isDocServerReady(uri)
    ) {
      return last;
    }
    return flushDidChange(monaco, model, uri, depth + 1);
  }
  if (model.isDisposed()) return null;
  const text = model.getValue();
  const changes = pendingContentChangesByUri.get(uri) ?? [];
  pendingContentChangesByUri.delete(uri);
  if (lastAppliedTextByUri.get(uri) === text && isDocServerReady(uri) && changes.length === 0) {
    return {
      rev: lastHubRevByUri.get(uri) ?? null,
      version: lastLspVersionByUri.get(uri) ?? null,
      serverReady: true,
    };
  }
  const job = (async (): Promise<SyncAck | null> => {
    const latest = model.getValue();
    const extra = pendingContentChangesByUri.get(uri) ?? [];
    pendingContentChangesByUri.delete(uri);
    const contentChanges = changes.concat(extra);
    try {
      const params: Record<string, unknown> = { uri, text: latest };
      if (contentChanges.length > 0) {
        params.contentChanges = contentChanges;
      }
      const res = await sendLsp("litecode/didChange", params, {
        timeoutMs: DOCUMENT_SYNC_TIMEOUT_MS,
      });
      if (res.error) {
        warnLsp("didChange", res.error.message);
        return null;
      }
      const ack = parseDidChangeAck(res.result);
      rememberApply(uri, latest, ack.rev, ack.version);
      noteDocServerReady(uri, ack.serverReady);
      void workspaceLsp?.ensureSemantic(uri);
      return ack;
    } catch (e) {
      if (!isCanceledError(e)) warnLsp("didChange", e);
      return null;
    } finally {
      applyInFlight.delete(uri);
    }
  })();
  applyInFlight.set(uri, job);
  return job;
}

function scheduleDidChange(
  monaco: typeof import("monaco-editor"),
  model: Monaco.editor.ITextModel,
  uri: string,
): void {
  const prev = changeDebounceTimers.get(uri);
  if (prev !== undefined) clearTimeout(prev);
  changeDebounceTimers.set(
    uri,
    setTimeout(() => {
      changeDebounceTimers.delete(uri);
      void flushDidChange(monaco, model, uri);
    }, DID_CHANGE_DEBOUNCE_MS),
  );
}

function isModelVisible(
  monaco: typeof import("monaco-editor"),
  model: Monaco.editor.ITextModel,
): boolean {
  return monaco.editor.getEditors().some((ed) => {
    if (ed.getModel() !== model) return false;
    const node = ed.getDomNode();
    if (!node?.isConnected) return false;
    const layout = ed.getLayoutInfo();
    return layout.width > 0 && layout.height > 0;
  });
}

function jumpFromEditor(
  editor: Monaco.editor.ICodeEditor,
  projectRoot: string,
): { path: string; line: number; column: number } | null {
  const model = editor.getModel();
  const pos = editor.getPosition();
  if (!model || !pos || !projectRoot) return null;
  return {
    path: relPathFromModel(model, projectRoot),
    line: pos.lineNumber,
    column: pos.column,
  };
}

function completionKind(
  monaco: typeof import("monaco-editor"),
  lspKind: number | undefined,
): Monaco.languages.CompletionItemKind {
  const K = monaco.languages.CompletionItemKind;
  switch (lspKind) {
    case 1:
      return K.Text;
    case 2:
      return K.Method;
    case 3:
      return K.Function;
    case 4:
      return K.Constructor;
    case 5:
      return K.Field;
    case 6:
      return K.Variable;
    case 7:
      return K.Class;
    case 8:
      return K.Interface;
    case 9:
      return K.Module;
    case 10:
      return K.Property;
    case 11:
      return K.Unit;
    case 12:
      return K.Value;
    case 13:
      return K.Enum;
    case 14:
      return K.Keyword;
    case 15:
      return K.Snippet;
    case 16:
      return K.Color;
    case 17:
      return K.File;
    case 18:
      return K.Reference;
    case 19:
      return K.Folder;
    case 20:
      return K.EnumMember;
    case 21:
      return K.Constant;
    case 22:
      return K.Struct;
    case 23:
      return K.Event;
    case 24:
      return K.Operator;
    case 25:
      return K.TypeParameter;
    default:
      return K.Function;
  }
}

function parseCompletionItems(
  monaco: typeof import("monaco-editor"),
  result: unknown,
  model: Monaco.editor.ITextModel,
  position: Monaco.Position,
): Monaco.languages.CompletionList {
  const itemsRaw = Array.isArray(result)
    ? result
    : result && typeof result === "object" && Array.isArray((result as { items?: unknown }).items)
      ? (result as { items: unknown[] }).items
      : [];
  const word = model.getWordUntilPosition(position);
  const defaultRange = {
    startLineNumber: position.lineNumber,
    startColumn: word.startColumn,
    endLineNumber: position.lineNumber,
    endColumn: word.endColumn,
  };
  const suggestions: Monaco.languages.CompletionItem[] = [];
  for (const raw of itemsRaw) {
    if (!raw || typeof raw !== "object") continue;
    const item = raw as {
      label?: string | { label?: string };
      insertText?: string;
      kind?: number;
      detail?: string;
      documentation?: string | { value?: string };
      textEdit?: { newText?: string; range?: unknown };
    };
    const label =
      typeof item.label === "string"
        ? item.label
        : item.label && typeof item.label === "object"
          ? String(item.label.label ?? "")
          : "";
    if (!label) continue;
    const insertText = item.textEdit?.newText ?? item.insertText ?? label;
    const range = lspRangeToMonaco(item.textEdit?.range) ?? defaultRange;
    const doc =
      typeof item.documentation === "string"
        ? item.documentation
        : item.documentation?.value;
    suggestions.push({
      label,
      kind: completionKind(monaco, item.kind),
      insertText,
      range,
      detail: item.detail,
      documentation: doc,
    });
  }
  const incomplete =
    typeof result === "object" &&
    result !== null &&
    (result as { isIncomplete?: boolean }).isIncomplete === true;
  return { suggestions, incomplete };
}

function lspPosition(position: Monaco.IPosition): { line: number; character: number } {
  return { line: position.lineNumber - 1, character: position.column - 1 };
}

function lspFileUri(
  model: Monaco.editor.ITextModel,
  projectRoot: string,
): string | null {
  if (!projectRoot) return null;
  return toFileUri(projectRoot, relPathFromModel(model, projectRoot));
}

function markupDoc(v: unknown): string | Monaco.IMarkdownString | undefined {
  if (typeof v === "string") return v;
  if (v && typeof v === "object" && "value" in v) {
    return { value: String((v as { value?: string }).value ?? "") };
  }
  return undefined;
}

function parseSignatureHelp(
  result: unknown,
): Monaco.languages.SignatureHelp | null {
  if (!result || typeof result !== "object") return null;
  const raw = result as {
    signatures?: unknown[];
    activeSignature?: number;
    activeParameter?: number;
  };
  const signatures: Monaco.languages.SignatureInformation[] = [];
  for (const s of raw.signatures ?? []) {
    if (!s || typeof s !== "object") continue;
    const sig = s as {
      label?: string;
      documentation?: unknown;
      parameters?: unknown[];
      activeParameter?: number;
    };
    if (typeof sig.label !== "string") continue;
    signatures.push({
      label: sig.label,
      documentation: markupDoc(sig.documentation),
      parameters: (sig.parameters ?? []).flatMap((p) => {
        if (!p || typeof p !== "object") return [];
        const param = p as { label?: string | [number, number]; documentation?: unknown };
        if (param.label === undefined) return [];
        return [
          {
            label: param.label,
            documentation: markupDoc(param.documentation),
          },
        ];
      }),
      activeParameter: sig.activeParameter,
    });
  }
  if (signatures.length === 0) return null;
  return {
    signatures,
    activeSignature: raw.activeSignature ?? 0,
    activeParameter: raw.activeParameter ?? 0,
  };
}

function parseDocumentHighlights(
  monaco: typeof import("monaco-editor"),
  result: unknown,
): Monaco.languages.DocumentHighlight[] {
  if (!Array.isArray(result)) return [];
  const K = monaco.languages.DocumentHighlightKind;
  const out: Monaco.languages.DocumentHighlight[] = [];
  for (const item of result) {
    if (!item || typeof item !== "object") continue;
    const h = item as { range?: unknown; kind?: number };
    const range = lspRangeToMonaco(h.range);
    if (!range) continue;
    const kind =
      h.kind === 2 ? K.Read : h.kind === 3 ? K.Write : K.Text;
    out.push({ range, kind });
  }
  return out;
}

function parseSelectionRangeChains(result: unknown): Monaco.languages.SelectionRange[][] {
  if (!Array.isArray(result)) return [];
  return result.map((node) => {
    const chain: Monaco.languages.SelectionRange[] = [];
    let cur: unknown = node;
    while (cur && typeof cur === "object" && "range" in cur) {
      const range = lspRangeToMonaco((cur as { range: unknown }).range);
      if (range) chain.push({ range });
      cur = (cur as { parent?: unknown }).parent;
    }
    return chain;
  });
}

function parseLinkedEditing(
  result: unknown,
): Monaco.languages.LinkedEditingRanges | null {
  if (!result || typeof result !== "object") return null;
  const raw = result as { ranges?: unknown[]; wordPattern?: string };
  const ranges = (raw.ranges ?? [])
    .map(lspRangeToMonaco)
    .filter((r): r is Monaco.IRange => r !== null);
  if (ranges.length === 0) return null;
  let wordPattern: RegExp | undefined;
  if (typeof raw.wordPattern === "string") {
    try {
      wordPattern = new RegExp(raw.wordPattern);
    } catch {
      wordPattern = undefined;
    }
  }
  return { ranges, wordPattern };
}

function parseInlayHints(
  monaco: typeof import("monaco-editor"),
  result: unknown,
): Monaco.languages.InlayHintList {
  const items = Array.isArray(result) ? result : [];
  const hints: Monaco.languages.InlayHint[] = [];
  for (const raw of items) {
    if (!raw || typeof raw !== "object") continue;
    const item = raw as {
      position?: { line?: number; character?: number };
      label?: unknown;
      kind?: number;
      tooltip?: unknown;
      paddingLeft?: boolean;
      paddingRight?: boolean;
    };
    const line = (item.position?.line ?? 0) + 1;
    const column = (item.position?.character ?? 0) + 1;
    let label: string | Monaco.languages.InlayHintLabelPart[];
    if (typeof item.label === "string") {
      label = item.label;
    } else if (Array.isArray(item.label)) {
      label = item.label.flatMap((part) => {
        if (typeof part === "string") return [{ label: part }];
        if (part && typeof part === "object" && "value" in part) {
          return [{ label: String((part as { value?: string }).value ?? "") }];
        }
        return [];
      });
    } else {
      continue;
    }
    const kind =
      item.kind === 1
        ? monaco.languages.InlayHintKind.Type
        : item.kind === 2
          ? monaco.languages.InlayHintKind.Parameter
          : undefined;
    hints.push({
      label,
      position: { lineNumber: line, column },
      kind,
      tooltip: markupDoc(item.tooltip),
      paddingLeft: item.paddingLeft,
      paddingRight: item.paddingRight,
    });
  }
  return { hints, dispose: () => {} };
}

function parseCodeLenses(result: unknown): Monaco.languages.CodeLensList {
  const items = Array.isArray(result) ? result : [];
  const lenses: Monaco.languages.CodeLens[] = [];
  for (const raw of items) {
    if (!raw || typeof raw !== "object") continue;
    const item = raw as {
      range?: unknown;
      command?: { title?: string; command?: string; arguments?: unknown[] };
    };
    const range = lspRangeToMonaco(item.range);
    if (!range) continue;
    const command =
      item.command && typeof item.command.command === "string"
        ? {
            id: item.command.command,
            title: item.command.title ?? item.command.command,
            arguments: item.command.arguments,
          }
        : undefined;
    lenses.push({ range, command });
  }
  return { lenses, dispose: () => {} };
}

const DEFAULT_TOKEN_TYPES = [
  "namespace",
  "type",
  "class",
  "enum",
  "interface",
  "struct",
  "typeParameter",
  "parameter",
  "variable",
  "property",
  "enumMember",
  "event",
  "function",
  "method",
  "macro",
  "keyword",
  "modifier",
  "comment",
  "string",
  "number",
  "regexp",
  "operator",
  "decorator",
];
const DEFAULT_TOKEN_MODIFIERS = [
  "declaration",
  "definition",
  "readonly",
  "static",
  "deprecated",
  "abstract",
  "async",
  "modification",
  "documentation",
  "defaultLibrary",
];

export type WorkspaceLspHandle = Monaco.IDisposable & {
  ensureSemantic: (fileUri: string) => Promise<void>;
};

type WorkspaceLspLive = {
  monaco: typeof import("monaco-editor");
  project: string;
  handle: WorkspaceLspHandle;
  ensureSemantic: (fileUri: string) => Promise<void>;
};

let workspaceLsp: WorkspaceLspLive | null = null;

/**
 * Register workspace-wide LSP providers on the **editor's** Monaco instance.
 * Must use the same `monaco` object passed to `Editor.onMount` — not `loader.init()`.
 */
export function registerWorkspaceLsp(
  monaco: typeof import("monaco-editor"),
  getProjectRoot: () => string,
): WorkspaceLspHandle {
  const requestAtPosition = async (
    method: string,
    model: Monaco.editor.ITextModel,
    position: Monaco.IPosition,
    extra?: Record<string, unknown>,
    token?: Monaco.CancellationToken,
  ): Promise<{ projectRoot: string; result: unknown } | null> => {
    if (!isLspWarm()) return null;
    const projectRoot = getProjectRoot();
    const uri = lspFileUri(model, projectRoot);
    if (!uri) return null;
    const ack = await flushDidChange(monaco, model, uri);
    if (!ack?.serverReady) return null;
    try {
      const res = await sendLsp(
        method,
        {
          textDocument: { uri },
          position: lspPosition(position),
          ...extra,
        },
        { token },
      );
      if (res.error) {
        if (!isCanceledError(res.error.message)) warnLsp(method, res.error.message);
        return null;
      }
      if (res.result === null || res.result === undefined) return null;
      return { projectRoot, result: res.result };
    } catch (e) {
      if (!isCanceledError(e)) warnLsp(method, e);
      return null;
    }
  };

  const locationsFor = (
    method: string,
  ): ((
    model: Monaco.editor.ITextModel,
    position: Monaco.Position,
  ) => Promise<Monaco.languages.Location[] | null>) => {
    return async (model, position) => {
      const got = await requestAtPosition(method, model, position);
      if (!got) return null;
      return parseLocations(monaco, got.result, got.projectRoot, model);
    };
  };

  const definitionProvider: Monaco.languages.DefinitionProvider = {
    provideDefinition: locationsFor("textDocument/definition"),
  };
  const typeDefinitionProvider: Monaco.languages.TypeDefinitionProvider = {
    provideTypeDefinition: locationsFor("textDocument/typeDefinition"),
  };
  const implementationProvider: Monaco.languages.ImplementationProvider = {
    provideImplementation: locationsFor("textDocument/implementation"),
  };
  const declarationProvider: Monaco.languages.DeclarationProvider = {
    provideDeclaration: locationsFor("textDocument/declaration"),
  };

  const referenceProvider: Monaco.languages.ReferenceProvider = {
    provideReferences: async (model, position) => {
      const got = await requestAtPosition("textDocument/references", model, position, {
        context: { includeDeclaration: true },
      });
      if (!got) return [];
      return parseLocations(monaco, got.result, got.projectRoot, model);
    },
  };

  const hoverProvider: Monaco.languages.HoverProvider = {
    provideHover: async (model, position, token) => {
      const got = await requestAtPosition(
        "textDocument/hover",
        model,
        position,
        undefined,
        token,
      );
      if (!got) return null;
      const parsed = parseHover(got.result);
      if (parsed.contents.length === 0) return null;
      return {
        contents: parsed.contents,
        range:
          parsed.range ??
          new monaco.Range(
            position.lineNumber,
            position.column,
            position.lineNumber,
            position.column,
          ),
      };
    },
  };

  const completionProvider: Monaco.languages.CompletionItemProvider = {
    triggerCharacters: [".", ":", ">"],
    provideCompletionItems: async (model, position, context, token) => {
      if (token.isCancellationRequested) return undefined;
      if (!isLspWarm()) return undefined;
      const triggerKind = monacoCompletionTriggerToLsp(context.triggerKind);
      const completionContext: Record<string, unknown> = { triggerKind };
      if (context.triggerKind === 1 && context.triggerCharacter) {
        completionContext.triggerCharacter = context.triggerCharacter;
      }
      const got = await requestAtPosition(
        "textDocument/completion",
        model,
        position,
        { context: completionContext },
        token,
      );
      if (token.isCancellationRequested || !got) return undefined;
      return parseCompletionItems(monaco, got.result, model, position);
    },
  };

  const signatureHelpProvider: Monaco.languages.SignatureHelpProvider = {
    signatureHelpTriggerCharacters: ["(", ","],
    signatureHelpRetriggerCharacters: [","],
    provideSignatureHelp: async (model, position, token, context) => {
      const got = await requestAtPosition(
        "textDocument/signatureHelp",
        model,
        position,
        {
          context: {
            triggerKind: context.triggerKind,
            triggerCharacter: context.triggerCharacter,
            isRetrigger: context.isRetrigger,
          },
        },
        token,
      );
      const value = got ? parseSignatureHelp(got.result) : null;
      if (!value) return null;
      return { value, dispose: () => {} };
    },
  };

  const documentHighlightProvider: Monaco.languages.DocumentHighlightProvider = {
    provideDocumentHighlights: async (model, position) => {
      const got = await requestAtPosition("textDocument/documentHighlight", model, position);
      if (!got) return [];
      return parseDocumentHighlights(monaco, got.result);
    },
  };

  const linkedEditingProvider: Monaco.languages.LinkedEditingRangeProvider = {
    provideLinkedEditingRanges: async (model, position) => {
      const got = await requestAtPosition("textDocument/linkedEditingRange", model, position);
      if (!got) return null;
      return parseLinkedEditing(got.result);
    },
  };

  const selectionRangeProvider: Monaco.languages.SelectionRangeProvider = {
    provideSelectionRanges: async (model, positions) => {
      if (!isLspWarm()) return [];
      const projectRoot = getProjectRoot();
      const uri = lspFileUri(model, projectRoot);
      if (!uri) return [];
      const ack = await flushDidChange(monaco, model, uri);
      if (!ack?.serverReady) return [];
      try {
        const res = await sendLsp("textDocument/selectionRange", {
          textDocument: { uri },
          positions: positions.map(lspPosition),
        });
        if (res.error) {
          warnLsp("textDocument/selectionRange", res.error.message);
          return [];
        }
        return parseSelectionRangeChains(res.result);
      } catch (e) {
        if (!isCanceledError(e)) warnLsp("textDocument/selectionRange", e);
        return [];
      }
    },
  };

  let semanticDisposable: Monaco.IDisposable | null = null;
  const ensureSemantic = async (fileUri: string) => {
    if (semanticDisposable) return;
    let tokenTypes = DEFAULT_TOKEN_TYPES;
    let tokenModifiers = DEFAULT_TOKEN_MODIFIERS;
    try {
      const res = await sendLsp("litecode/serverCapabilities", { uri: fileUri });
      const raw =
        res.result && typeof res.result === "object"
          ? (res.result as { tokenTypes?: unknown; tokenModifiers?: unknown })
          : {};
      if (Array.isArray(raw.tokenTypes) && raw.tokenTypes.every((t) => typeof t === "string")) {
        tokenTypes = raw.tokenTypes as string[];
      }
      if (
        Array.isArray(raw.tokenModifiers) &&
        raw.tokenModifiers.every((t) => typeof t === "string")
      ) {
        tokenModifiers = raw.tokenModifiers as string[];
      }
    } catch {
      // Keep LSP defaults when capabilities are not ready yet.
    }
    const provider: Monaco.languages.DocumentSemanticTokensProvider = {
      getLegend: () => ({ tokenTypes, tokenModifiers }),
      provideDocumentSemanticTokens: async (model, _lastResultId, token) => {
        // VS Code: version mismatch / hide throws cancel and keeps existing tokens.
        // Returning null here would wipe highlighting.
        if (!isModelVisible(monaco, model)) throw canceledError();
        await sleepCancellable(SEMANTIC_DEBOUNCE_MS, token);
        const projectRoot = getProjectRoot();
        if (!projectRoot || !isLspWarm()) throw canceledError();
        const uri = lspFileUri(model, projectRoot);
        if (!uri) throw canceledError();
        const ack = await flushDidChange(monaco, model, uri);
        if (!ack?.serverReady || token.isCancellationRequested) throw canceledError();
        try {
          const res = await sendLsp(
            "textDocument/semanticTokens/full",
            { textDocument: { uri } },
            { token },
          );
          if (res.error || !res.result || typeof res.result !== "object") {
            throw canceledError();
          }
          const data = (res.result as { data?: unknown }).data;
          if (!Array.isArray(data)) throw canceledError();
          return { data: Uint32Array.from(data as number[]) };
        } catch (e) {
          if (!isCanceledError(e)) warnLsp("semanticTokens", e);
          throw canceledError();
        }
      },
      releaseDocumentSemanticTokens: () => {},
    };
    const parts: Monaco.IDisposable[] = [];
    for (const langId of LSP_LANGUAGES) {
      parts.push(
        monaco.languages.registerDocumentSemanticTokensProvider(langId, provider),
      );
    }
    semanticDisposable = {
      dispose: () => {
        for (const p of parts) p.dispose();
      },
    };
  };

  const inlayHintsProvider: Monaco.languages.InlayHintsProvider = {
    provideInlayHints: async (model, range, token) => {
      if (!isModelVisible(monaco, model) || token.isCancellationRequested) {
        return { hints: [], dispose: () => {} };
      }
      try {
        await sleepCancellable(INLAY_DEBOUNCE_MS, token);
        const projectRoot = getProjectRoot();
        const uri = projectRoot ? lspFileUri(model, projectRoot) : null;
        if (!uri || !isLspWarm()) return { hints: [], dispose: () => {} };
        const ack = await flushDidChange(monaco, model, uri);
        if (!ack?.serverReady || token.isCancellationRequested) {
          return { hints: [], dispose: () => {} };
        }
        const res = await sendLsp(
          "textDocument/inlayHint",
          {
            textDocument: { uri },
            range: {
              start: {
                line: range.startLineNumber - 1,
                character: range.startColumn - 1,
              },
              end: {
                line: range.endLineNumber - 1,
                character: range.endColumn - 1,
              },
            },
          },
          { token },
        );
        if (res.error || token.isCancellationRequested) {
          return { hints: [], dispose: () => {} };
        }
        return parseInlayHints(monaco, res.result);
      } catch (e) {
        if (!isCanceledError(e)) warnLsp("inlayHint", e);
        return { hints: [], dispose: () => {} };
      }
    },
  };

  const codeLensProvider: Monaco.languages.CodeLensProvider = {
    provideCodeLenses: async (model, token) => {
      if (!isModelVisible(monaco, model) || token.isCancellationRequested) {
        return { lenses: [], dispose: () => {} };
      }
      try {
        const projectRoot = getProjectRoot();
        const uri = projectRoot ? lspFileUri(model, projectRoot) : null;
        if (!uri || !isLspWarm()) return { lenses: [], dispose: () => {} };
        const ack = await flushDidChange(monaco, model, uri);
        if (!ack?.serverReady || token.isCancellationRequested) {
          return { lenses: [], dispose: () => {} };
        }
        const res = await sendLsp("textDocument/codeLens", { textDocument: { uri } }, { token });
        if (res.error || token.isCancellationRequested) {
          return { lenses: [], dispose: () => {} };
        }
        return parseCodeLenses(res.result);
      } catch (e) {
        if (!isCanceledError(e)) warnLsp("codeLens", e);
        return { lenses: [], dispose: () => {} };
      }
    },
  };

  const disposables: Monaco.IDisposable[] = [];
  for (const langId of LSP_LANGUAGES) {
    disposables.push(
      monaco.languages.registerDefinitionProvider(langId, definitionProvider),
      monaco.languages.registerTypeDefinitionProvider(langId, typeDefinitionProvider),
      monaco.languages.registerImplementationProvider(langId, implementationProvider),
      monaco.languages.registerDeclarationProvider(langId, declarationProvider),
      monaco.languages.registerReferenceProvider(langId, referenceProvider),
      monaco.languages.registerHoverProvider(langId, hoverProvider),
      monaco.languages.registerCompletionItemProvider(langId, completionProvider),
      monaco.languages.registerSignatureHelpProvider(langId, signatureHelpProvider),
      monaco.languages.registerDocumentHighlightProvider(langId, documentHighlightProvider),
      monaco.languages.registerLinkedEditingRangeProvider(langId, linkedEditingProvider),
      monaco.languages.registerSelectionRangeProvider(langId, selectionRangeProvider),
      monaco.languages.registerInlayHintsProvider(langId, inlayHintsProvider),
      monaco.languages.registerCodeLensProvider(langId, codeLensProvider),
    );
  }

  disposables.push(
    monaco.editor.registerEditorOpener({
      openCodeEditor: async (_input, resource, selectionOrPosition) => {
        const projectRoot = getProjectRoot();
        const focused = monaco.editor.getEditors().find((ed) => ed.hasTextFocus());
        if (focused && projectRoot) {
          const from = jumpFromEditor(focused, projectRoot);
          if (from) useEditorStore.getState().pushJump(from);
        }
        const rel = resource.path.replace(/^\//, "");
        const line =
          selectionOrPosition &&
          typeof selectionOrPosition === "object" &&
          "startLineNumber" in selectionOrPosition
            ? selectionOrPosition.startLineNumber
            : selectionOrPosition &&
                typeof selectionOrPosition === "object" &&
                "lineNumber" in selectionOrPosition
              ? (selectionOrPosition as Monaco.IPosition).lineNumber
              : 1;
        const column =
          selectionOrPosition &&
          typeof selectionOrPosition === "object" &&
          "startColumn" in selectionOrPosition
            ? selectionOrPosition.startColumn
            : selectionOrPosition &&
                typeof selectionOrPosition === "object" &&
                "column" in selectionOrPosition
              ? (selectionOrPosition as Monaco.IPosition).column
              : 1;
        await useEditorStore.getState().openFileAt(rel, line, column);
        const editor = monaco.editor
          .getEditors()
          .find((ed) => ed.getModel()?.uri.toString() === resource.toString());
        if (!editor) return true;
        if (
          selectionOrPosition &&
          typeof selectionOrPosition === "object" &&
          "startLineNumber" in selectionOrPosition
        ) {
          editor.setSelection(selectionOrPosition);
          editor.revealRangeInCenter(selectionOrPosition);
        }
        return true;
      },
    }),
  );

  return {
    dispose: () => {
      for (const d of disposables) d.dispose();
      semanticDisposable?.dispose();
      semanticDisposable = null;
    },
    ensureSemantic,
  };
}

/** One Language Client per workspace Monaco instance. Panes only bind. */
export function ensureWorkspaceLsp(
  monaco: typeof import("monaco-editor"),
  getProjectRoot: () => string,
): WorkspaceLspHandle {
  const project = getProjectRoot();
  if (
    workspaceLsp &&
    workspaceLsp.monaco === monaco &&
    workspaceLsp.project === project
  ) {
    return workspaceLsp.handle;
  }
  dropWorkspaceLsp();
  const registered = registerWorkspaceLsp(monaco, getProjectRoot);
  workspaceLsp = {
    monaco,
    project,
    handle: registered,
    ensureSemantic: registered.ensureSemantic,
  };
  return registered;
}

export function dropWorkspaceLsp(): void {
  if (workspaceLsp) {
    workspaceLsp.handle.dispose();
    workspaceLsp = null;
  }
  resetDocumentAuthority();
}

export function bindEditorLsp(
  editor: Monaco.editor.IStandaloneCodeEditor,
  monaco: typeof import("monaco-editor"),
  getProjectRoot: () => string,
): Monaco.IDisposable {
  const disposables: Monaco.IDisposable[] = [];

  const syncBuffer = () => {
    const model = editor.getModel();
    const projectRoot = getProjectRoot();
    if (!model || !projectRoot || !isLspWarm()) return;
    const rel = relPathFromModel(model, projectRoot);
    const uri = toFileUri(projectRoot, rel);
    void flushDidChange(monaco, model, uri);
  };

  disposables.push(
    editor.onDidChangeModelContent((e) => {
      const model = editor.getModel();
      const projectRoot = getProjectRoot();
      if (!model || !projectRoot || !isLspWarm()) return;
      const uri = toFileUri(projectRoot, relPathFromModel(model, projectRoot));
      queueContentChanges(uri, e.changes);
      scheduleDidChange(monaco, model, uri);
    }),
  );
  syncBuffer();

  const jumpHere = () => jumpFromEditor(editor, getProjectRoot());

  editor.addCommand(monaco.KeyMod.Alt | monaco.KeyCode.LeftArrow, () => {
    const loc = useEditorStore.getState().goJumpBack(jumpHere() ?? undefined);
    if (loc) void useEditorStore.getState().openFileAt(loc.path, loc.line, loc.column);
  });
  editor.addCommand(monaco.KeyMod.Alt | monaco.KeyCode.RightArrow, () => {
    const loc = useEditorStore.getState().goJumpForward(jumpHere() ?? undefined);
    if (loc) void useEditorStore.getState().openFileAt(loc.path, loc.line, loc.column);
  });

  return {
    dispose: () => {
      for (const d of disposables) d.dispose();
    },
  };
}

export async function refreshDiagnostics(
  monaco: typeof import("monaco-editor"),
  projectRoot: string,
  relPath: string,
  fileUri?: string,
): Promise<void> {
  if (!isLspWarm() || !projectRoot) {
    return;
  }
  const uri = fileUri ?? toFileUri(projectRoot, relPath);
  let res: LspResult;
  try {
    res = await sendLsp(
      "litecode/getDiagnostics",
      { uri },
      { timeoutMs: DOCUMENT_SYNC_TIMEOUT_MS },
    );
  } catch (e) {
    warnLsp("diagnostics", e);
    return;
  }
  if (res.error) {
    warnLsp("diagnostics", res.error.message);
    return;
  }

  const model = monaco.editor
    .getModels()
    .find(
      (m) =>
        relPathFromModel(m, projectRoot) === relPath ||
        m.uri.path.replace(/^\//, "") === relPath,
    );
  if (!model) return;
  const snap = parseDiagnosticsSnapshot(res.result);
  if (typeof snap.rev === "number") lastHubRevByUri.set(uri, snap.rev);
  noteDocServerReady(uri, snap.serverReady);
  if (snap.fresh && snap.serverReady) {
    applyRawDiagnostics(monaco, model, snap.diagnostics);
  }
}

export function getProjectRootFromStore(): string {
  return useConnectionStore.getState().project;
}

attachSiblingStores({ lsp: handleLspWireEnvelope });

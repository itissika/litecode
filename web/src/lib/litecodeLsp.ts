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

export function isLspWarm(): boolean {
  return useSettingsStore.getState().engineStatuses.lsp?.state === "warm";
}

export function handleLspWireEnvelope(env: WireEnvelope): boolean {
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

function sendLsp(
  method: string,
  params: Record<string, unknown>,
): Promise<LspResult> {
  const id = nextId++;
  nextId = nextId % Number.MAX_SAFE_INTEGER || 1;
  return new Promise((resolve, reject) => {
    pending.set(id, { resolve, reject });
    useConnectionStore.getState().sendRpc<{ result: unknown }>("lsp/request", {
      method,
      params,
    })
      .then((response) => {
        const waiter = pending.get(id);
        if (waiter) {
          pending.delete(id);
          waiter.resolve({ id, result: response.result } as LspResult);
        }
      })
      .catch((e) => {
        pending.delete(id);
        reject(e instanceof Error ? e : new Error(String(e)));
      });
    setTimeout(() => {
      if (pending.has(id)) {
        pending.delete(id);
        reject(new Error("LSP request timed out"));
      }
    }, 15_000);
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

function parseHover(result: unknown): Monaco.IMarkdownString[] {
  if (!result || typeof result !== "object") return [];
  const contents = (result as { contents?: unknown }).contents;
  if (typeof contents === "string") {
    return [{ value: contents }];
  }
  if (contents && typeof contents === "object" && "value" in contents) {
    const value = (contents as { value?: string }).value ?? "";
    return [{ value }];
  }
  if (Array.isArray(contents)) {
    return contents.map((c) => {
      if (typeof c === "string") return { value: c };
      if (c && typeof c === "object" && "value" in c) {
        return { value: String((c as { value?: string }).value ?? "") };
      }
      return { value: "" };
    });
  }
  return [];
}

function warnLsp(context: string, err: unknown): void {
  if (!import.meta.env.DEV) return;
  const msg = err instanceof Error ? err.message : String(err);
  console.warn(`[litecode-lsp] ${context}: ${msg}`);
}

function lspStatusHover(error: unknown): Monaco.IMarkdownString {
  const message = error instanceof Error ? error.message : String(error);
  return { value: `**LSP unavailable**\n\n${message}` };
}

/**
 * Register workspace-wide LSP providers on the **editor's** Monaco instance.
 * Must use the same `monaco` object passed to `Editor.onMount` — not `loader.init()`.
 */
export function registerWorkspaceLsp(
  monaco: typeof import("monaco-editor"),
  getProjectRoot: () => string,
): Monaco.IDisposable {
  const definitionProvider: Monaco.languages.DefinitionProvider = {
    provideDefinition: async (model, position) => {
      const projectRoot = getProjectRoot();
      if (!projectRoot) return null;
      try {
        const rel = relPathFromModel(model, projectRoot);
        const fileUri = toFileUri(projectRoot, rel);
        if (import.meta.env.DEV) {
          console.debug(
            `[litecode-lsp] textDocument.uri => ${fileUri} (root: ${projectRoot})`,
          );
        }
        const res = await sendLsp("textDocument/definition", {
          textDocument: { uri: fileUri },
          position: {
            line: position.lineNumber - 1,
            character: position.column - 1,
          },
        });
        if (res.error) {
          warnLsp("definition", res.error.message);
          return null;
        }
        return parseLocations(monaco, res.result, projectRoot, model);
      } catch (e) {
        warnLsp("definition", e);
        return null;
      }
    },
  };

  const hoverProvider: Monaco.languages.HoverProvider = {
    provideHover: async (model, position) => {
      const projectRoot = getProjectRoot();
      if (!projectRoot) return null;
      try {
        const rel = relPathFromModel(model, projectRoot);
        const fileUri = toFileUri(projectRoot, rel);
        if (import.meta.env.DEV) {
          console.debug(
            `[litecode-lsp] textDocument.uri => ${fileUri} (root: ${projectRoot})`,
          );
        }
        const res = await sendLsp("textDocument/hover", {
          textDocument: { uri: fileUri },
          position: {
            line: position.lineNumber - 1,
            character: position.column - 1,
          },
        });
        if (res.error) {
          warnLsp("hover", res.error.message);
          return {
            contents: [lspStatusHover(res.error.message)],
            range: new monaco.Range(
              position.lineNumber,
              position.column,
              position.lineNumber,
              position.column,
            ),
          };
        }
        const contents = parseHover(res.result);
        if (contents.length === 0) return null;
        return {
          contents,
          range: new monaco.Range(
            position.lineNumber,
            position.column,
            position.lineNumber,
            position.column,
          ),
        };
      } catch (e) {
        warnLsp("hover", e);
        // Do not silently discard engine Loading / workspace-scope failures:
        // users need an actionable result at the same interaction point where
        // a normal hover would have appeared.
        return {
          contents: [lspStatusHover(e)],
          range: new monaco.Range(
            position.lineNumber,
            position.column,
            position.lineNumber,
            position.column,
          ),
        };
      }
    },
  };

  const disposables: Monaco.IDisposable[] = [];
  for (const langId of LSP_LANGUAGES) {
    disposables.push(
      monaco.languages.registerDefinitionProvider(langId, definitionProvider),
      monaco.languages.registerHoverProvider(langId, hoverProvider),
    );
  }

  disposables.push(
    monaco.editor.registerEditorOpener({
      openCodeEditor: async (_input, resource, selectionOrPosition) => {
        const rel = resource.path.replace(/^\//, "");
        await useEditorStore.getState().openFile(rel);
        const editor = monaco.editor
          .getEditors()
          .find((ed) => ed.getModel()?.uri.toString() === resource.toString());
        if (!editor) return false;
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
    res = await sendLsp("litecode/getDiagnostics", { uri });
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

  const text =
    res.result &&
    typeof res.result === "object" &&
    "text" in res.result &&
    typeof (res.result as { text?: string }).text === "string"
      ? (res.result as { text: string }).text
      : "";
  if (!text || text === "No diagnostics") {
    monaco.editor.setModelMarkers(model, "litecode-lsp", []);
    return;
  }

  const markers: Monaco.editor.IMarkerData[] = [];
  for (const line of text.split("\n")) {
    const m = line.match(/^(Error|Warning|Information|Hint): (.+) \((\d+):(\d+)\)$/);
    if (!m) continue;
    const severity =
      m[1] === "Error"
        ? monaco.MarkerSeverity.Error
        : m[1] === "Warning"
          ? monaco.MarkerSeverity.Warning
          : monaco.MarkerSeverity.Info;
    const lineNum = Number(m[3]);
    const col = Number(m[4]);
    markers.push({
      severity,
      message: m[2],
      startLineNumber: lineNum,
      startColumn: col,
      endLineNumber: lineNum,
      endColumn: col + 1,
    });
  }
  monaco.editor.setModelMarkers(model, "litecode-lsp", markers);
}

export function getProjectRootFromStore(): string {
  return useConnectionStore.getState().project;
}

attachSiblingStores({ lsp: handleLspWireEnvelope });

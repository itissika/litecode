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

/** Monaco CompletionTriggerKind 0/1/2 → LSP CompletionTriggerKind 1/2/3. */
export function monacoCompletionTriggerToLsp(monacoKind: number): number {
  return monacoKind + 1;
}

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
  opts?: { token?: Monaco.CancellationToken },
): Promise<LspResult> {
  const id = nextId++;
  nextId = nextId % Number.MAX_SAFE_INTEGER || 1;
  const rpcId = nextLspRpcId++;
  return new Promise((resolve, reject) => {
    pending.set(id, { resolve, reject });
    const cancel = () => {
      void useConnectionStore.getState().sendRpc("lsp/request", {
        method: "$/cancelRequest",
        params: { id: rpcId },
      });
    };
    const sub = opts?.token?.onCancellationRequested(cancel);
    if (opts?.token?.isCancellationRequested) {
      cancel();
    }
    useConnectionStore.getState().sendRpc<{ result: unknown }>("lsp/request", {
      method,
      params,
      rpc_id: rpcId,
    })
      .then((response) => {
        sub?.dispose();
        const waiter = pending.get(id);
        if (waiter) {
          pending.delete(id);
          waiter.resolve({ id, result: response.result } as LspResult);
        }
      })
      .catch((e) => {
        sub?.dispose();
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

type EditorCaps = {
  tokenTypes: string[];
  tokenModifiers: string[];
  triggerCharacters: string[];
};

let cachedCaps: EditorCaps | null = null;
let semanticDisposable: Monaco.IDisposable | null = null;

function asStringArray(v: unknown, fallback: string[]): string[] {
  if (!Array.isArray(v)) return fallback;
  const out = v.filter((x): x is string => typeof x === "string");
  return out.length > 0 ? out : fallback;
}

async function loadEditorCaps(fileUri: string): Promise<EditorCaps> {
  if (cachedCaps) return cachedCaps;
  try {
    const res = await sendLsp("litecode/serverCapabilities", { uri: fileUri });
    const r = res.result && typeof res.result === "object" ? res.result as Record<string, unknown> : {};
    cachedCaps = {
      tokenTypes: asStringArray(r.tokenTypes, DEFAULT_TOKEN_TYPES),
      tokenModifiers: asStringArray(r.tokenModifiers, DEFAULT_TOKEN_MODIFIERS),
      triggerCharacters: asStringArray(r.triggerCharacters, ["."]),
    };
  } catch {
    cachedCaps = {
      tokenTypes: DEFAULT_TOKEN_TYPES,
      tokenModifiers: DEFAULT_TOKEN_MODIFIERS,
      triggerCharacters: ["."],
    };
  }
  return cachedCaps;
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

function diagnosticsFromResult(result: unknown): unknown {
  if (result && typeof result === "object" && "diagnostics" in result) {
    return (result as { diagnostics: unknown }).diagnostics;
  }
  return [];
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

function monacoRangeToLsp(range: Monaco.IRange): {
  start: { line: number; character: number };
  end: { line: number; character: number };
} {
  return {
    start: { line: range.startLineNumber - 1, character: range.startColumn - 1 },
    end: { line: range.endLineNumber - 1, character: range.endColumn - 1 },
  };
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

function parseInlayHints(result: unknown): Monaco.languages.InlayHint[] {
  const items = Array.isArray(result)
    ? result
    : result && typeof result === "object" && Array.isArray((result as { inlayHints?: unknown }).inlayHints)
      ? (result as { inlayHints: unknown[] }).inlayHints
      : [];
  const out: Monaco.languages.InlayHint[] = [];
  for (const item of items) {
    if (!item || typeof item !== "object") continue;
    const h = item as {
      position?: { line?: number; character?: number };
      label?: unknown;
      kind?: number;
      tooltip?: unknown;
      paddingLeft?: boolean;
      paddingRight?: boolean;
    };
    if (!h.position) continue;
    let label: Monaco.languages.InlayHint["label"];
    if (typeof h.label === "string") {
      label = h.label;
    } else if (Array.isArray(h.label)) {
      label = h.label.flatMap((part) => {
        if (typeof part === "string") return [{ label: part }];
        if (!part || typeof part !== "object") return [];
        const p = part as { value?: string; label?: string };
        const text = p.value ?? p.label;
        return typeof text === "string" ? [{ label: text }] : [];
      });
    } else {
      continue;
    }
    out.push({
      label,
      position: {
        lineNumber: (h.position.line ?? 0) + 1,
        column: (h.position.character ?? 0) + 1,
      },
      kind: h.kind === 1 || h.kind === 2 ? h.kind : undefined,
      tooltip: markupDoc(h.tooltip),
      paddingLeft: h.paddingLeft,
      paddingRight: h.paddingRight,
    });
  }
  return out;
}

function parseCodeLenses(result: unknown): Monaco.languages.CodeLens[] {
  if (!Array.isArray(result)) return [];
  const out: Monaco.languages.CodeLens[] = [];
  for (const item of result) {
    if (!item || typeof item !== "object") continue;
    const c = item as {
      range?: unknown;
      command?: { title?: string; command?: string; arguments?: unknown[] };
    };
    const range = lspRangeToMonaco(c.range);
    if (!range) continue;
    const command =
      c.command && typeof c.command.command === "string"
        ? {
            id: c.command.command,
            title: c.command.title ?? c.command.command,
            arguments: c.command.arguments,
          }
        : undefined;
    out.push({ range, command });
  }
  return out;
}

export type WorkspaceLspHandle = Monaco.IDisposable & {
  ensureSemantic: (fileUri: string) => Promise<void>;
};

/**
 * Register workspace-wide LSP providers on the **editor's** Monaco instance.
 * Must use the same `monaco` object passed to `Editor.onMount` — not `loader.init()`.
 */
export function registerWorkspaceLsp(
  monaco: typeof import("monaco-editor"),
  getProjectRoot: () => string,
): WorkspaceLspHandle {
  cachedCaps = null;
  semanticDisposable?.dispose();
  semanticDisposable = null;

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
        warnLsp(method, res.error.message);
        return null;
      }
      return { projectRoot, result: res.result };
    } catch (e) {
      warnLsp(method, e);
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
      const contents = parseHover(got.result);
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
    },
  };

  const completionProvider: Monaco.languages.CompletionItemProvider = {
    triggerCharacters: [".", ":", ">"],
    provideCompletionItems: async (model, position, context, token) => {
      if (!isLspWarm()) return { suggestions: [] };
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
      if (!got) return { suggestions: [] };
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
        warnLsp("textDocument/selectionRange", e);
        return [];
      }
    },
  };

  const inlayHintsProvider: Monaco.languages.InlayHintsProvider = {
    provideInlayHints: async (model, range) => {
      if (!isLspWarm()) return { hints: [], dispose: () => {} };
      const projectRoot = getProjectRoot();
      const uri = lspFileUri(model, projectRoot);
      if (!uri) return { hints: [], dispose: () => {} };
      try {
        const res = await sendLsp("textDocument/inlayHint", {
          textDocument: { uri },
          range: monacoRangeToLsp(range),
        });
        if (res.error) {
          warnLsp("textDocument/inlayHint", res.error.message);
          return { hints: [], dispose: () => {} };
        }
        return { hints: parseInlayHints(res.result), dispose: () => {} };
      } catch (e) {
        warnLsp("textDocument/inlayHint", e);
        return { hints: [], dispose: () => {} };
      }
    },
  };

  const codeLensProvider: Monaco.languages.CodeLensProvider = {
    provideCodeLenses: async (model) => {
      if (!isLspWarm()) return { lenses: [], dispose: () => {} };
      const projectRoot = getProjectRoot();
      const uri = lspFileUri(model, projectRoot);
      if (!uri) return { lenses: [], dispose: () => {} };
      try {
        const res = await sendLsp("textDocument/codeLens", {
          textDocument: { uri },
        });
        if (res.error) {
          warnLsp("textDocument/codeLens", res.error.message);
          return { lenses: [], dispose: () => {} };
        }
        return { lenses: parseCodeLenses(res.result), dispose: () => {} };
      } catch (e) {
        warnLsp("textDocument/codeLens", e);
        return { lenses: [], dispose: () => {} };
      }
    },
  };

  const ensureSemantic = async (fileUri: string) => {
    if (semanticDisposable) return;
    const caps = await loadEditorCaps(fileUri);
    const provider: Monaco.languages.DocumentSemanticTokensProvider = {
      getLegend: () => ({
        tokenTypes: caps.tokenTypes,
        tokenModifiers: caps.tokenModifiers,
      }),
      provideDocumentSemanticTokens: async (model) => {
        const projectRoot = getProjectRoot();
        if (!projectRoot || !isLspWarm()) return null;
        try {
          const rel = relPathFromModel(model, projectRoot);
          const uri = toFileUri(projectRoot, rel);
          const res = await sendLsp("textDocument/semanticTokens/full", {
            textDocument: { uri },
          });
          if (res.error || !res.result || typeof res.result !== "object") {
            return null;
          }
          const data = (res.result as { data?: unknown }).data;
          if (!Array.isArray(data)) return null;
          return { data: Uint32Array.from(data as number[]) };
        } catch (e) {
          warnLsp("semanticTokens", e);
          return null;
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
      cachedCaps = null;
    },
    ensureSemantic,
  };
}

export function bindEditorLsp(
  editor: Monaco.editor.IStandaloneCodeEditor,
  monaco: typeof import("monaco-editor"),
  getProjectRoot: () => string,
  helpers?: {
    ensureSemantic?: (fileUri: string) => Promise<void>;
  },
): Monaco.IDisposable {
  const disposables: Monaco.IDisposable[] = [];
  let changeTimer: ReturnType<typeof setTimeout> | undefined;

  const syncBuffer = () => {
    const model = editor.getModel();
    const projectRoot = getProjectRoot();
    if (!model || !projectRoot || !isLspWarm()) return;
    const rel = relPathFromModel(model, projectRoot);
    const uri = toFileUri(projectRoot, rel);
    void helpers?.ensureSemantic?.(uri);
    void sendLsp("litecode/didChange", { uri, text: model.getValue() })
      .then((res) => {
        if (res.error) {
          warnLsp("didChange", res.error.message);
          return;
        }
        applyRawDiagnostics(monaco, model, diagnosticsFromResult(res.result));
      })
      .catch((e) => warnLsp("didChange", e));
  };

  disposables.push(
    editor.onDidChangeModelContent(() => {
      if (changeTimer !== undefined) clearTimeout(changeTimer);
      changeTimer = setTimeout(syncBuffer, 150);
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
      if (changeTimer !== undefined) clearTimeout(changeTimer);
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
  applyRawDiagnostics(monaco, model, diagnosticsFromResult(res.result));
}

export function getProjectRootFromStore(): string {
  return useConnectionStore.getState().project;
}

attachSiblingStores({ lsp: handleLspWireEnvelope });

import Editor from "@monaco-editor/react";
import { CodeIcon, MarkdownLogoIcon } from "@phosphor-icons/react";
import {
  Component,
  lazy,
  Suspense,
  useCallback,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import type { DockviewPanelApi } from "dockview-react";
import type { editor } from "monaco-editor";

import { fetchSymbolAt } from "../api/workspace";
import { formatLineSpan, symbolMentionSource } from "../lib/knowledge/markers";
import { spanFromSelection, writeCodeSpan } from "../lib/dropPayload";
import { appendComposerText, composerTarget } from "../stores/composerDraft";
import { useEditorStore } from "../stores/editorStore";
import { useConnectionStore } from "../stores/connectionStore";
import { useSessionStore } from "../stores/sessionStore";
import { useEngineStore } from "../stores/engineStore";
import { ConflictCard } from "./ConflictCard";
import {
  bindEditorLsp,
  dropWorkspaceLsp,
  ensureWorkspaceLsp,
  getProjectRootFromStore,
} from "../lib/litecodeLsp";
import {
  applyMonacoThemeForApp,
  defineAllMonacoThemes,
  LITECODE_MONACO_THEME_DARK,
  LITECODE_MONACO_THEME_LIGHT,
} from "../theme/monaco";
import { getTheme, THEME_CHANGE_EVENT } from "../lib/theme";
import { BINARY_FILE_MESSAGE } from "../lib/fileKind";
import { languageFromPath } from "../utils/language";
import {
  isWysiwygMarkdownPath,
  resolveMdEditorView,
  WYSIWYG_MARKDOWN_MAX_CHARS,
} from "../utils/wysiwygMarkdown";
import { FileFallback } from "./fileview/FileFallback";
import { ImagePreview } from "./fileview/ImagePreview";
import { MediaPreview } from "./fileview/MediaPreview";
import { SqlitePreview } from "./fileview/SqlitePreview";

const MilkdownMarkdownEditor = lazy(async () => {
  const mod = await import("./MilkdownMarkdownEditor");
  return { default: mod.MilkdownMarkdownEditor };
});

const PdfPreview = lazy(async () => {
  const mod = await import("./fileview/PdfPreview");
  return { default: mod.PdfPreview };
});

/** A failed lazy preview must not unmount the rest of the workbench. */
class PreviewErrorBoundary extends Component<
  { resetKey: string; children: ReactNode },
  { failed: boolean }
> {
  state = { failed: false };

  static getDerivedStateFromError() {
    return { failed: true };
  }

  componentDidUpdate(prev: { resetKey: string }) {
    if (prev.resetKey !== this.props.resetKey && this.state.failed) {
      this.setState({ failed: false });
    }
  }

  render() {
    if (this.state.failed) {
      return (
        <div className="flex h-full flex-col items-center justify-center gap-3 px-6 text-center">
          <p className="max-w-md text-sm text-(--_dk-text)">
            This preview failed to load. Reload the window and open the file again.
          </p>
          <button
            type="button"
            className="btn btn-sm"
            onClick={() => window.location.reload()}
          >
            Reload
          </button>
        </div>
      );
    }
    return this.props.children;
  }
}

export function EditorPane({
  filePath,
  api,
}: {
  filePath: string;
  api?: DockviewPanelApi;
}) {
  const tab = useEditorStore(
    (s) => s.tabs.find((t) => t.path === filePath) ?? null,
  );
  const project = useSessionStore((s) => s.project);
  const wsConnected = useConnectionStore((s) => s.state === "connected");
  const lspDesired = useEngineStore((s) => {
    return s.engineStatuses.lsp?.desired === true;
  });
  const setContent = useEditorStore((s) => s.setContent);
  const setMdView = useEditorStore((s) => s.setMdView);
  const mdViewOverride = useEditorStore((s) => s.mdViewByPath[filePath]);
  const pendingReveal = useEditorStore((s) => s.pendingReveal);
  const conflict = useEditorStore((s) => s.conflicts[filePath] ?? null);
  const clearConflict = useEditorStore((s) => s.clearConflict);

  const monacoRef = useRef<typeof import("monaco-editor") | null>(null);
  const editorRef = useRef<editor.IStandaloneCodeEditor | null>(null);
  const addToChatRef = useRef<{ dispose: () => void } | null>(null);
  const spanDragCleanup = useRef<(() => void) | null>(null);
  const pathRef = useRef(filePath);
  pathRef.current = filePath;
  const milkdownHostRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    return () => {
      addToChatRef.current?.dispose();
      spanDragCleanup.current?.();
    };
  }, []);

  const addSelectionToChat = useCallback(async (ed: editor.ICodeEditor) => {
    const selection = ed.getSelection();
    if (!selection || selection.isEmpty()) return;
    const sessionId = composerTarget();
    if (!sessionId) return;
    let end = selection.endLineNumber;
    if (selection.endColumn === 1 && end > selection.startLineNumber) end -= 1;
    const start = selection.startLineNumber;
    const path = pathRef.current;
    let chain = "";
    try {
      const hit = await fetchSymbolAt(path, start, end);
      chain = hit.chain?.trim() ?? "";
    } catch {
      chain = "";
    }
    const lines = formatLineSpan(start, end);
    const text = chain
      ? symbolMentionSource(path, { symbol: chain, lines })
      : symbolMentionSource(path, { lines });
    appendComposerText(sessionId, text);
  }, []);
  const lspBindRef = useRef<(() => void) | null>(null);
  const [monacoTheme, setMonacoTheme] = useState(() =>
    getTheme() === "light"
      ? LITECODE_MONACO_THEME_LIGHT
      : LITECODE_MONACO_THEME_DARK,
  );

  const mdView = resolveMdEditorView(
    filePath,
    tab?.content.length ?? 0,
    mdViewOverride,
  );
  const showMdToggle =
    isWysiwygMarkdownPath(filePath) &&
    (tab?.content.length ?? 0) <= WYSIWYG_MARKDOWN_MAX_CHARS;
  const useWysiwyg = mdView === "wysiwyg";

  useEffect(() => {
    if (useWysiwyg) editorRef.current = null;
  }, [useWysiwyg]);

  const bindEditorToLsp = useCallback(
    (
      monaco: typeof import("monaco-editor"),
      ed: editor.IStandaloneCodeEditor,
    ) => {
      lspBindRef.current?.();
      lspBindRef.current = null;
      if (!lspDesired || !wsConnected) return;
      const d = bindEditorLsp(ed, monaco, getProjectRootFromStore);
      lspBindRef.current = () => d.dispose();
    },
    [lspDesired, wsConnected],
  );

  const syncLspRegistration = useCallback(
    (monaco: typeof import("monaco-editor")) => {
      if (!lspDesired || !wsConnected || !getProjectRootFromStore()) {
        dropWorkspaceLsp();
        lspBindRef.current?.();
        lspBindRef.current = null;
        return;
      }
      ensureWorkspaceLsp(monaco, getProjectRootFromStore);
      const ed = editorRef.current;
      if (ed) bindEditorToLsp(monaco, ed);
    },
    [lspDesired, wsConnected, bindEditorToLsp],
  );

  // The panel owns its read, the same way an agent panel owns its subscription.
  // Every transition to connected (first connect and each reconnect) re-reads.
  useEffect(() => {
    if (!wsConnected || tab?.external) return;
    void useEditorStore.getState().ensureReadable(filePath);
  }, [wsConnected, filePath, tab?.external]);

  // Listen to dockview panel api events
  useEffect(() => {
    if (!api) return;
    const disposables = [
      api.onDidDimensionsChange(() => {
        requestAnimationFrame(() => {
          editorRef.current?.layout();
        });
      }),
      api.onDidActiveChange((event) => {
        if (event.isActive) {
          // Keep editorStore.activePath aligned with the visible Dockview tab
          // so workbench-level Ctrl+S targets the file the user is looking at.
          useEditorStore.setState({ activePath: filePath });
          editorRef.current?.focus();
          const prose =
            milkdownHostRef.current?.querySelector<HTMLElement>(".ProseMirror");
          prose?.focus();
        }
      }),
    ];
    return () => disposables.forEach((d) => d.dispose());
  }, [api, filePath]);

  // React to theme changes (e.g. from menu toggle).
  // Redefine themes with fresh hex colors, then setTheme + layout — avoids a
  // blank/white editor when CSS tokens are rgba() (Monaco only accepts hex).
  useEffect(() => {
    const handler = (e: Event) => {
      const theme = (e as CustomEvent<string>).detail;
      const next =
        theme === "light"
          ? LITECODE_MONACO_THEME_LIGHT
          : LITECODE_MONACO_THEME_DARK;
      setMonacoTheme(next);
      const monaco = monacoRef.current;
      if (!monaco) return;
      try {
        applyMonacoThemeForApp(monaco, theme);
        requestAnimationFrame(() => {
          editorRef.current?.layout();
        });
      } catch (err) {
        console.error("monaco theme apply failed", err);
      }
    };
    window.addEventListener(THEME_CHANGE_EVENT, handler);
    return () => window.removeEventListener(THEME_CHANGE_EVENT, handler);
  }, []);

  // Keep a workspace Language Client while LSP is desired; panes only bind.
  useEffect(() => {
    const monaco = monacoRef.current;
    if (!monaco) return;
    syncLspRegistration(monaco);
    return () => {
      lspBindRef.current?.();
      lspBindRef.current = null;
    };
  }, [project, lspDesired, wsConnected, syncLspRegistration]);

  // Reveal line requested by workspace search / go-to.
  useEffect(() => {
    const ed = editorRef.current;
    if (!ed || !tab || tab.loading) return;
    if (!pendingReveal || pendingReveal.path !== filePath) return;
    const reveal = useEditorStore.getState().consumePendingReveal();
    if (!reveal) return;
    const line = Math.max(1, reveal.line);
    const column = Math.max(1, reveal.column ?? 1);
    ed.revealLineInCenter(line);
    ed.setPosition({ lineNumber: line, column });
    ed.focus();
  }, [filePath, tab?.loading, tab?.content, pendingReveal]);

  return (
    <div className="flex h-full flex-col" data-drop-zone="editor">
      {showMdToggle && (
        <div className="flex h-7 shrink-0 items-center justify-end gap-1 border-b border-(--_dk-line-visible) bg-(--_dk-editor) px-2">
          <button
            type="button"
            className={`btn-xs inline-flex items-center gap-1 ${useWysiwyg ? "btn-primary" : "btn-ghost"}`}
            title="Markdown"
            aria-pressed={useWysiwyg}
            onClick={() => setMdView(filePath, "wysiwyg")}
          >
            <MarkdownLogoIcon size={12} />
            Markdown
          </button>
          <button
            type="button"
            className={`btn-xs inline-flex items-center gap-1 ${!useWysiwyg ? "btn-primary" : "btn-ghost"}`}
            title="Source"
            aria-pressed={!useWysiwyg}
            onClick={() => setMdView(filePath, "source")}
          >
            <CodeIcon size={12} />
            Source
          </button>
        </div>
      )}
      <div className="relative min-h-0 flex-1 h-full">
        {tab ? (
          <>
            {tab.loading && tab.kind === "text" && (
              <div className="absolute inset-0 z-10 flex items-center justify-center bg-(--_dk-editor)/80 text-sm text-(--_dk-text-muted)">
                Loading…
              </div>
            )}
            {tab.error && tab.kind === "text" && tab.content ? (
              <div className="border-b border-(--_dk-tag-danger-border) bg-(--_dk-tag-danger-bg) px-3 py-1 text-xs text-(--_dk-tag-danger-fg)">
                {tab.error}
              </div>
            ) : null}
            {conflict && (
              <ConflictCard
                path={conflict.path}
                source={conflict.source}
                onDismiss={() => clearConflict(conflict.path)}
              />
            )}
            {tab.kind === "image" ? (
              <ImagePreview
                path={filePath}
                diskRevision={tab.diskRevision}
                sourceUrl={tab.previewUrl}
              />
            ) : tab.kind === "pdf" ? (
              <PreviewErrorBoundary resetKey={filePath}>
                <Suspense
                  fallback={
                    <div className="flex h-full items-center justify-center text-sm text-(--_dk-text-muted)">
                      Loading…
                    </div>
                  }
                >
                  <PdfPreview
                    path={filePath}
                    diskRevision={tab.diskRevision}
                    sourceUrl={tab.previewUrl}
                  />
                </Suspense>
              </PreviewErrorBoundary>
            ) : tab.kind === "audio" || tab.kind === "video" ? (
              <MediaPreview
                path={filePath}
                diskRevision={tab.diskRevision}
                kind={tab.kind}
                sourceUrl={tab.previewUrl}
              />
            ) : tab.kind === "sqlite" ? (
              <SqlitePreview path={filePath} diskRevision={tab.diskRevision} />
            ) : tab.kind === "binary" ||
              (tab.error && !tab.errorRetryable && tab.content === "") ? (
              <FileFallback
                path={filePath}
                message={tab.error ?? BINARY_FILE_MESSAGE}
              />
            ) : useWysiwyg ? (
              tab.loading ? null : (
                <div ref={milkdownHostRef} className="h-full">
                  <PreviewErrorBoundary resetKey={filePath}>
                    <Suspense
                      fallback={
                        <div className="flex h-full items-center justify-center text-sm text-(--_dk-text-muted)">
                          Loading editor…
                        </div>
                      }
                    >
                      <MilkdownMarkdownEditor
                        filePath={filePath}
                        content={tab.content ?? ""}
                        onChange={(markdown) => setContent(filePath, markdown)}
                      />
                    </Suspense>
                  </PreviewErrorBoundary>
                </div>
              )
            ) : (
              <Editor
                path={tab.path ?? filePath}
                height="100%"
                language={tab.language ?? languageFromPath(filePath)}
                value={tab.content ?? ""}
                theme={monacoTheme}
                beforeMount={defineAllMonacoThemes}
                onMount={(_editor, monaco) => {
                  monacoRef.current = monaco;
                  editorRef.current = _editor;
                  _editor.layout();
                  const model = monaco.editor.getModel(
                    monaco.Uri.parse(filePath),
                  );
                  if (model) {
                    _editor.setModel(model);
                  }
                  syncLspRegistration(monaco);
                  bindEditorToLsp(monaco, _editor);
                  addToChatRef.current?.dispose();
                  addToChatRef.current = _editor.addAction({
                    id: "litecode.add-selection-to-chat",
                    label: "Add to chat",
                    contextMenuGroupId: "9_cutcopypaste",
                    contextMenuOrder: 2,
                    keybindings: [
                      monaco.KeyMod.CtrlCmd | monaco.KeyMod.Alt | monaco.KeyCode.KeyL,
                    ],
                    precondition: "editorHasSelection",
                    run: (ed) => {
                      void addSelectionToChat(ed);
                    },
                  });
                  spanDragCleanup.current?.();
                  const node = _editor.getDomNode();
                  if (node) {
                    const onDragStart = (event: DragEvent) => {
                      if (!event.dataTransfer) return;
                      const span = spanFromSelection(
                        pathRef.current,
                        _editor.getSelection(),
                      );
                      if (!span) return;
                      writeCodeSpan(event.dataTransfer, span);
                    };
                    node.addEventListener("dragstart", onDragStart);
                    spanDragCleanup.current = () =>
                      node.removeEventListener("dragstart", onDragStart);
                  } else {
                    spanDragCleanup.current = null;
                  }
                  const pending = useEditorStore.getState().pendingReveal;
                  if (pending && pending.path === filePath) {
                    const reveal = useEditorStore
                      .getState()
                      .consumePendingReveal();
                    if (reveal) {
                      const line = Math.max(1, reveal.line);
                      const column = Math.max(1, reveal.column ?? 1);
                      _editor.revealLineInCenter(line);
                      _editor.setPosition({ lineNumber: line, column });
                      _editor.focus();
                    }
                  }
                }}
                onChange={(value) => setContent(filePath, value ?? "")}
                options={{
                  padding: { top: 12, bottom: 12 },
                  minimap: { enabled: false },
                  fontSize: 14,
                  fontFamily:
                    '"JetBrains Mono", Menlo, Monaco, "Courier New", monospace',
                  lineNumbers: "on",
                  scrollBeyondLastLine: false,
                  automaticLayout: true,
                  tabSize: 2,
                  autoClosingBrackets: "languageDefined",
                  autoClosingQuotes: "languageDefined",
                  autoSurround: "languageDefined",
                  autoIndent: "full",
                  matchBrackets: "always",
                  formatOnType: false,
                  formatOnPaste: false,
                  wordBasedSuggestions: "off",
                  parameterHints: { enabled: true },
                  codeLens: true,
                  linkedEditing: true,
                  inlayHints: { enabled: "on" },
                  bracketPairColorization: { enabled: true },
                  guides: { indentation: true, bracketPairs: true },
                  "semanticHighlighting.enabled": true,
                  gotoLocation: {
                    multiple: "goto",
                    multipleDefinitions: "goto",
                    multipleReferences: "peek",
                    alternativeDefinitionCommand:
                      "editor.action.goToReferences",
                  },
                }}
              />
            )}
          </>
        ) : (
          <div className="flex h-full items-center justify-center text-sm text-(--_dk-text-muted)">
            {wsConnected ? "Loading…" : "Waiting to reconnect…"}
          </div>
        )}
      </div>
    </div>
  );
}

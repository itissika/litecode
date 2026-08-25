import Editor from "@monaco-editor/react";
import { CodeIcon, MarkdownLogoIcon } from "@phosphor-icons/react";
import { lazy, Suspense, useCallback, useEffect, useRef, useState } from "react";
import type { DockviewPanelApi } from "dockview-react";
import type { editor } from "monaco-editor";

import { useEditorStore } from "../stores/editorStore";
import { useConnectionStore } from "../stores/connectionStore";
import { useSessionStore } from "../stores/sessionStore";
import { useSettingsStore } from "../stores/settingsStore";
import { ConflictCard } from "./ConflictCard";
import {
  bindEditorLsp,
  getProjectRootFromStore,
  registerWorkspaceLsp,
  refreshDiagnostics,
  type WorkspaceLspHandle,
} from "../lib/litecodeLsp";
import {
  applyMonacoThemeForApp,
  defineAllMonacoThemes,
  LITECODE_MONACO_THEME_DARK,
  LITECODE_MONACO_THEME_LIGHT,
} from "../theme/monaco";
import { getTheme, THEME_CHANGE_EVENT } from "../lib/theme";
import { languageFromPath } from "../utils/language";
import {
  isWysiwygMarkdownPath,
  resolveMdEditorView,
  WYSIWYG_MARKDOWN_MAX_CHARS,
} from "../utils/wysiwygMarkdown";

const MilkdownMarkdownEditor = lazy(async () => {
  const mod = await import("./MilkdownMarkdownEditor");
  return { default: mod.MilkdownMarkdownEditor };
});

export function EditorPane({ filePath, api }: { filePath: string; api?: DockviewPanelApi }) {
  const tab = useEditorStore((s) => s.tabs.find((t) => t.path === filePath) ?? null);
  const saving = useEditorStore((s) => s.saving);
  const project = useSessionStore((s) => s.project);
  const wsConnected = useConnectionStore(
    (s) => s.state === "connected",
  );
  const lspReady = useSettingsStore((s) => {
    return s.engineStatuses.lsp?.state === "warm";
  });
  const lspDesired = useSettingsStore((s) => {
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
  const milkdownHostRef = useRef<HTMLDivElement | null>(null);
  const lspDisposeRef = useRef<(() => void) | null>(null);
  const lspBindRef = useRef<(() => void) | null>(null);
  const lspHelpersRef = useRef<Pick<WorkspaceLspHandle, "ensureSemantic"> | null>(
    null,
  );
  const [monacoTheme, setMonacoTheme] = useState(
    () => getTheme() === "light" ? LITECODE_MONACO_THEME_LIGHT : LITECODE_MONACO_THEME_DARK
  );

  const mdView = resolveMdEditorView(
    filePath,
    tab?.content.length ?? 0,
    mdViewOverride,
  );
  const showMdToggle = isWysiwygMarkdownPath(filePath)
    && (tab?.content.length ?? 0) <= WYSIWYG_MARKDOWN_MAX_CHARS;
  const useWysiwyg = mdView === "wysiwyg";

  useEffect(() => {
    if (useWysiwyg) editorRef.current = null;
  }, [useWysiwyg]);

  const bindEditorToLsp = useCallback(
    (monaco: typeof import("monaco-editor"), ed: editor.IStandaloneCodeEditor) => {
      lspBindRef.current?.();
      lspBindRef.current = null;
      const helpers = lspHelpersRef.current;
      if (!helpers || !lspDesired || !wsConnected) return;
      const d = bindEditorLsp(ed, monaco, getProjectRootFromStore, helpers);
      lspBindRef.current = () => d.dispose();
    },
    [lspDesired, wsConnected],
  );

  const syncLspRegistration = useCallback(
    (monaco: typeof import("monaco-editor")) => {
      lspDisposeRef.current?.();
      lspDisposeRef.current = null;
      lspHelpersRef.current = null;
      // Register as soon as LSP is desired, not only once it is Warm. The
      // workspace RPC then reports a visible Loading / Unavailable status
      // instead of silently omitting hover while the engine starts.
      if (!lspDesired || !wsConnected || !getProjectRootFromStore()) {
        return;
      }
      const disposable = registerWorkspaceLsp(monaco, getProjectRootFromStore);
      lspHelpersRef.current = {
        ensureSemantic: disposable.ensureSemantic,
      };
      lspDisposeRef.current = () => disposable.dispose();
      const ed = editorRef.current;
      if (ed) bindEditorToLsp(monaco, ed);
    },
    [lspDesired, wsConnected, bindEditorToLsp],
  );

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
          const prose = milkdownHostRef.current?.querySelector<HTMLElement>(
            ".ProseMirror",
          );
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
        theme === "light" ? LITECODE_MONACO_THEME_LIGHT : LITECODE_MONACO_THEME_DARK;
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

  // Re-register when hub becomes warm or workspace root changes
  useEffect(() => {
    const monaco = monacoRef.current;
    if (!monaco) return;
    syncLspRegistration(monaco);
    return () => {
      lspDisposeRef.current?.();
      lspDisposeRef.current = null;
      lspBindRef.current?.();
      lspBindRef.current = null;
    };
  }, [project, lspDesired, wsConnected, syncLspRegistration]);

  useEffect(() => {
    const monaco = monacoRef.current;
    if (!monaco || !tab || !project || saving || !lspReady) return;
    if (tab.dirty) return;
    void refreshDiagnostics(monaco, project, tab.path);
  }, [tab?.path, tab?.dirty, tab?.savedContent, project, saving, lspReady]);

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
    <div className="flex h-full flex-col">
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
            {tab.loading && (
              <div className="absolute inset-0 z-10 flex items-center justify-center bg-(--_dk-editor)/80 text-sm text-(--_dk-text-muted)">
                Loading…
              </div>
            )}
            {tab.error && (
              <div className="border-b border-(--_dk-red-500) bg-(--_dk-red-500) px-3 py-1 text-xs text-(--_dk-red-500)">
                {tab.error}
              </div>
            )}
            {conflict && (
              <ConflictCard
                path={conflict.path}
                source={conflict.source}
                onDismiss={() => clearConflict(conflict.path)}
              />
            )}
            {useWysiwyg ? (
              tab.loading ? null : (
              <div ref={milkdownHostRef} className="h-full">
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
                const model = monaco.editor.getModel(monaco.Uri.parse(filePath));
                if (model) {
                  _editor.setModel(model);
                }
                syncLspRegistration(monaco);
                bindEditorToLsp(monaco, _editor);
                if (project && lspReady && !tab.dirty) {
                  void refreshDiagnostics(monaco, project, filePath);
                }
                const pending = useEditorStore.getState().pendingReveal;
                if (pending && pending.path === filePath) {
                  const reveal = useEditorStore.getState().consumePendingReveal();
                  if (reveal) {
                    const line = Math.max(1, reveal.line);
                    const column = Math.max(1, reveal.column ?? 1);
                    _editor.revealLineInCenter(line);
                    _editor.setPosition({ lineNumber: line, column });
                    _editor.focus();
                  }
                }
              }}
              onChange={(value) =>
                setContent(filePath, value ?? "")
              }
              options={{
                padding: { top: 12, bottom: 12 },
                minimap: { enabled: false },
                fontSize: 14,
                fontFamily: '"JetBrains Mono", Menlo, Monaco, "Courier New", monospace',
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
                linkedEditing: true,
                inlayHints: { enabled: "on" },
                bracketPairColorization: { enabled: true },
                guides: { indentation: true, bracketPairs: true },
                "semanticHighlighting.enabled": true,
                gotoLocation: {
                  multiple: "goto",
                  multipleDefinitions: "goto",
                  multipleReferences: "peek",
                  alternativeDefinitionCommand: "editor.action.goToReferences",
                },
              }}
            />
            )}
          </>
        ) : (
          <div className="flex h-full items-center justify-center text-sm text-(--_dk-text-disabled)">
            Open a file from the explorer
          </div>
        )}
      </div>
    </div>
  );
}

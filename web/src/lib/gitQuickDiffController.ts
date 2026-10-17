import type { editor, Uri } from "monaco-editor";

import { gitShow, type GitShowRev } from "../api/workspace";
import { THEME_CHANGE_EVENT } from "./theme";
import { useGitStore } from "../stores/gitStore";
import {
  MAX_QUICK_DIFF_BYTES,
  PEEK_HEADER_PX,
  baselineLabel,
  decorationRange,
  dropOverlappingSecondary,
  exceedsQuickDiffLimit,
  glyphClassName,
  hunkAtLine,
  hunksForBaseline,
  hunksFromChanges,
  lineIntersectsHunk,
  modifiedSpan,
  originalsForPath,
  peekAfterLine,
  peekHeightPx,
  stagedLookupPath,
  stagedOriginalText,
  type DiffBaseline,
  type DiffHunk,
  type DiffKind,
} from "./gitQuickDiff";

const DIFF_DEBOUNCE_MS = 300;

export interface GitQuickDiffHandle {
  refresh: () => void;
  dispose: () => void;
}

type Monaco = typeof import("monaco-editor");

/**
 * Gutter bars for the open file, plus the inline diff that opens from a bar.
 * Originals come from `git show`; the live editor model is the modified side.
 */
export function attachGitQuickDiff(
  codeEditor: editor.IStandaloneCodeEditor,
  monaco: Monaco,
  getPath: () => string,
): GitQuickDiffHandle {
  let disposed = false;
  let generation = 0;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let latest: DiffHunk[] = [];
  let statusSeen = useGitStore.getState().status;
  const blobCache = new Map<string, string | null>();
  const decorations = codeEditor.createDecorationsCollection();

  const host = document.createElement("div");
  host.setAttribute("aria-hidden", "true");
  host.style.cssText =
    "position:fixed;left:-10000px;top:0;width:8px;height:8px;overflow:hidden;opacity:0;pointer-events:none;";
  document.body.appendChild(host);
  const hidden = monaco.editor.createDiffEditor(host, {
    automaticLayout: false,
    readOnly: true,
    renderSideBySide: true,
    renderIndicators: false,
    renderOverviewRuler: false,
    renderMarginRevertIcon: false,
    renderGutterMenu: false,
    minimap: { enabled: false },
    scrollbar: { vertical: "hidden", horizontal: "hidden" },
    overviewRulerLanes: 0,
    hideUnchangedRegions: { enabled: false },
    maxComputationTime: 2000,
    ignoreTrimWhitespace: true,
  });

  const peek = new QuickDiffPeek(codeEditor, monaco, () => latest, getPath);

  const schedule = () => {
    if (disposed) return;
    if (timer) clearTimeout(timer);
    timer = setTimeout(() => {
      timer = null;
      const token = ++generation;
      void refresh(token);
    }, DIFF_DEBOUNCE_MS);
  };

  const unsubStatus = useGitStore.subscribe((state, prev) => {
    if (state.status !== prev.status) schedule();
  });

  const onTheme = () => {
    if (!disposed) paint(latest);
  };
  window.addEventListener(THEME_CHANGE_EVENT, onTheme);

  const contentSub = codeEditor.onDidChangeModelContent(schedule);
  const modelSub = codeEditor.onDidChangeModel(() => {
    peek.close();
    schedule();
  });
  const disposeSub = codeEditor.onDidDispose(() => dispose());

  let downLine: number | null = null;
  const downSub = codeEditor.onMouseDown((event) => {
    if (!event.event.leftButton) {
      downLine = null;
      return;
    }
    downLine = glyphLine(monaco, event);
  });
  const upSub = codeEditor.onMouseUp((event) => {
    const line = downLine;
    downLine = null;
    if (line == null || glyphLine(monaco, event) !== line) return;
    const hunk = hunkAtLine(latest, line);
    if (!hunk) return;
    if (peek.isShowing(hunk)) peek.close();
    else peek.show(hunk, line, true);
  });
  const keySub = codeEditor.onKeyDown((event) => {
    if (!peek.isOpen()) return;
    if (event.keyCode !== monaco.KeyCode.Escape) return;
    event.preventDefault();
    event.stopPropagation();
    peek.close();
  });

  schedule();

  function paint(hunks: DiffHunk[]) {
    latest = hunks;
    const model = codeEditor.getModel();
    if (!model || model.isDisposed()) {
      decorations.clear();
      peek.close();
      return;
    }
    const lineCount = model.getLineCount();
    decorations.set(
      hunks.map((hunk) => ({
        range: decorationRange(hunk, lineCount),
        options: {
          description: "git-quick-diff",
          isWholeLine: hunk.kind !== "delete",
          linesDecorationsClassName: glyphClassName(hunk),
          overviewRuler: {
            color: rulerColor(hunk.kind, hunk.baseline),
            position: monaco.editor.OverviewRulerLane.Left,
          },
        },
      })),
    );
    peek.sync();
  }

  function clearBars() {
    paint([]);
  }

  async function refresh(token: number) {
    if (disposed || token !== generation) return;
    const model = codeEditor.getModel();
    const path = getPath();
    if (
      !model ||
      model.isDisposed() ||
      exceedsQuickDiffLimit(model.getValueLength())
    ) {
      clearBars();
      return;
    }
    const status = currentStatus();
    const sources = originalsForPath(status, path);
    if (!sources.worktree && !sources.staged) {
      clearBars();
      return;
    }

    const language = model.getLanguageId();
    let worktreeText: string | null = null;
    let stagedText: string | null = null;
    if (sources.worktree) {
      worktreeText = await loadBlob(sources.worktree.path, "index");
      if (disposed || token !== generation) return;
      if (
        worktreeText != null &&
        exceedsQuickDiffLimit(worktreeText.length)
      ) {
        worktreeText = null;
      }
    }
    if (sources.staged) {
      const head = await loadBlob(stagedLookupPath(sources.staged), "HEAD");
      if (disposed || token !== generation) return;
      stagedText = stagedOriginalText(sources.staged, head);
      if (stagedText != null && exceedsQuickDiffLimit(stagedText.length)) {
        stagedText = null;
      }
    }
    if (worktreeText == null && stagedText == null) {
      clearBars();
      return;
    }

    const collected: DiffHunk[] = [];
    if (worktreeText != null) {
      const original = upsertModel(
        monaco,
        blobUri(monaco, "index", path),
        worktreeText,
        language,
      );
      const changes = await diffAgainst(hidden, original, model);
      if (disposed || token !== generation) return;
      collected.push(...hunksFromChanges("worktree", changes));
    }
    if (stagedText != null) {
      const original = upsertModel(
        monaco,
        blobUri(monaco, "head", stagedLookupPath(sources.staged!)),
        stagedText,
        language,
      );
      const changes = await diffAgainst(hidden, original, model);
      if (disposed || token !== generation) return;
      collected.push(...hunksFromChanges("staged", changes));
    }
    paint(dropOverlappingSecondary(collected));
  }

  function currentStatus() {
    const status = useGitStore.getState().status;
    if (status !== statusSeen) {
      blobCache.clear();
      statusSeen = status;
    }
    return status;
  }

  async function loadBlob(path: string, rev: GitShowRev): Promise<string | null> {
    const key = `${rev}\n${path}`;
    const cached = blobCache.get(key);
    if (cached !== undefined) return cached;
    try {
      const content = await gitShow(path, rev);
      if (content != null && content.length > MAX_QUICK_DIFF_BYTES) {
        blobCache.set(key, null);
        return null;
      }
      blobCache.set(key, content);
      return content;
    } catch {
      return null;
    }
  }

  function dispose() {
    if (disposed) return;
    disposed = true;
    generation += 1;
    if (timer) clearTimeout(timer);
    unsubStatus();
    window.removeEventListener(THEME_CHANGE_EVENT, onTheme);
    contentSub.dispose();
    modelSub.dispose();
    disposeSub.dispose();
    downSub.dispose();
    upSub.dispose();
    keySub.dispose();
    decorations.clear();
    peek.dispose();
    hidden.dispose();
    host.remove();
  }

  return { refresh: schedule, dispose };
}

function glyphLine(
  monaco: Monaco,
  event: editor.IEditorMouseEvent,
): number | null {
  if (event.target.type !== monaco.editor.MouseTargetType.GUTTER_LINE_DECORATIONS) {
    return null;
  }
  const element = event.target.element;
  const glyph = element?.closest(".dirty-diff-glyph");
  if (!glyph) return null;
  const rect = glyph.getBoundingClientRect();
  const x = event.event.browserEvent.clientX;
  if (x < rect.left - 3 || x > rect.right + 3) return null;
  return event.target.range?.startLineNumber ?? null;
}

async function diffAgainst(
  diffEditor: editor.IStandaloneDiffEditor,
  original: editor.ITextModel,
  modified: editor.ITextModel,
): Promise<editor.ILineChange[]> {
  diffEditor.setModel(null);
  return new Promise((resolve) => {
    let settled = false;
    let timer = 0;
    const finish = () => {
      if (settled) return;
      const current = diffEditor.getModel();
      if (
        !current ||
        current.original !== original ||
        current.modified !== modified
      ) {
        return;
      }
      const changes = diffEditor.getLineChanges();
      if (!changes) return;
      settled = true;
      window.clearTimeout(timer);
      sub.dispose();
      resolve(changes);
    };
    const sub = diffEditor.onDidUpdateDiff(finish);
    diffEditor.setModel({ original, modified });
    timer = window.setTimeout(() => {
      if (settled) return;
      settled = true;
      sub.dispose();
      resolve(diffEditor.getLineChanges() ?? []);
    }, 5000);
  });
}

function blobUri(monaco: Monaco, kind: "index" | "head", path: string): Uri {
  return monaco.Uri.from({
    scheme: "git-show",
    path: `/${kind}/${path.replace(/\\/g, "/")}`,
  });
}

function upsertModel(
  monaco: Monaco,
  uri: Uri,
  value: string,
  language: string,
): editor.ITextModel {
  const existing = monaco.editor.getModel(uri);
  if (existing && !existing.isDisposed()) {
    if (existing.getLanguageId() !== language) {
      monaco.editor.setModelLanguage(existing, language);
    }
    if (existing.getValue() !== value) existing.setValue(value);
    return existing;
  }
  return monaco.editor.createModel(value, language, uri);
}

function readToken(name: string, fallback: string): string {
  if (typeof document === "undefined") return fallback;
  const value = getComputedStyle(document.documentElement)
    .getPropertyValue(name)
    .trim();
  return value || fallback;
}

function withAlpha(color: string, alpha: number): string {
  const hex = color.trim();
  if (!/^#[0-9a-fA-F]{6}$/.test(hex)) return color;
  const r = Number.parseInt(hex.slice(1, 3), 16);
  const g = Number.parseInt(hex.slice(3, 5), 16);
  const b = Number.parseInt(hex.slice(5, 7), 16);
  return `rgba(${r}, ${g}, ${b}, ${alpha})`;
}

function kindColor(kind: DiffKind): string {
  if (kind === "add") return readToken("--_dk-cat-green", "#22c55e");
  if (kind === "delete") return readToken("--_dk-red-500", "#ef4444");
  return readToken("--_dk-cat-blue", "#1a85ff");
}

function rulerColor(kind: DiffKind, baseline: DiffBaseline): string {
  const color = kindColor(kind);
  return baseline === "staged" ? withAlpha(color, 0.55) : color;
}

class QuickDiffPeek {
  private open = false;
  private hunk: DiffHunk | null = null;
  private anchorLine = 1;
  private zoneId: string | null = null;
  private zoneHeight = 0;
  private zoneAfter = -1;
  private readonly panel: HTMLElement;
  private readonly title: HTMLElement;
  private readonly body: HTMLElement;
  private readonly diffEditor: editor.IStandaloneDiffEditor;
  private readonly zoneSpacer: HTMLElement;
  private revealHunk: DiffHunk | null = null;
  private readonly subscriptions: Array<{ dispose: () => void }> = [];

  constructor(
    private readonly editor: editor.IStandaloneCodeEditor,
    private readonly monaco: Monaco,
    private readonly hunks: () => readonly DiffHunk[],
    private readonly getPath: () => string,
  ) {
    this.panel = document.createElement("div");
    this.panel.className = "git-quick-diff-peek";
    this.panel.style.visibility = "hidden";
    this.panel.setAttribute("role", "dialog");
    this.panel.addEventListener("keydown", (event) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      event.stopPropagation();
      this.close();
      this.editor.focus();
    });
    this.panel.addEventListener(
      "wheel",
      (event) => {
        event.stopPropagation();
      },
      { passive: true },
    );

    const header = document.createElement("div");
    header.className = "git-quick-diff-peek-header";
    this.title = document.createElement("span");
    this.title.className = "git-quick-diff-peek-title";
    const actions = document.createElement("span");
    actions.className = "git-quick-diff-peek-actions";
    actions.append(
      this.button("上一处", () => this.step(-1)),
      this.button("下一处", () => this.step(1)),
      this.button("关闭", () => this.close()),
    );
    header.append(this.title, actions);

    this.body = document.createElement("div");
    this.body.className = "git-quick-diff-peek-body";
    this.panel.append(header, this.body);
    document.body.appendChild(this.panel);

    const fontSize = editor.getOption(monaco.editor.EditorOption.fontSize);
    const fontFamily = editor.getOption(monaco.editor.EditorOption.fontFamily);
    this.diffEditor = monaco.editor.createDiffEditor(this.body, {
      automaticLayout: false,
      readOnly: true,
      originalEditable: false,
      renderSideBySide: false,
      renderIndicators: false,
      renderOverviewRuler: false,
      renderMarginRevertIcon: false,
      renderGutterMenu: false,
      minimap: { enabled: false },
      scrollBeyondLastLine: false,
      stickyScroll: { enabled: false },
      hideUnchangedRegions: { enabled: false },
      ignoreTrimWhitespace: true,
      fontSize,
      fontFamily,
      wordWrap: "off",
      folding: false,
      glyphMargin: false,
      overviewRulerLanes: 0,
      scrollbar: {
        vertical: "auto",
        horizontal: "auto",
        verticalScrollbarSize: 10,
        horizontalScrollbarSize: 10,
      },
      padding: { top: 4, bottom: 4 },
    });
    this.subscriptions.push(
      this.diffEditor.onDidUpdateDiff(() => {
        const hunk = this.revealHunk;
        if (!hunk) return;
        this.revealHunk = null;
        const line = Math.max(1, modifiedSpan(hunk).start);
        this.diffEditor.getModifiedEditor().revealLineInCenter(line);
      }),
    );
    const modified = this.diffEditor.getModifiedEditor();
    modified.addCommand(monaco.KeyCode.Escape, () => {
      this.close();
      this.editor.focus();
    });

    this.zoneSpacer = document.createElement("div");
    this.subscriptions.push(editor.onDidScrollChange(() => this.position()));
    this.subscriptions.push(editor.onDidLayoutChange(() => this.position()));
  }

  isOpen(): boolean {
    return this.open;
  }

  isShowing(hunk: DiffHunk): boolean {
    return this.open && this.hunk != null && sameHunk(this.hunk, hunk);
  }

  show(hunk: DiffHunk, anchorLine: number, reveal: boolean) {
    const model = this.editor.getModel();
    if (!model) return;
    this.open = true;
    this.hunk = hunk;
    this.anchorLine = anchorLine;
    this.renderChrome(hunk);
    this.placeZone(hunk);
    this.bindModels(hunk, model, reveal);
    this.position();
    requestAnimationFrame(() => this.position());
  }

  sync() {
    if (!this.open || !this.hunk) return;
    const next = hunksForBaseline(this.hunks(), this.hunk.baseline).find((hunk) =>
      lineIntersectsHunk(this.anchorLine, hunk),
    );
    if (!next) {
      this.close();
      return;
    }
    const moved = !sameHunk(this.hunk, next);
    this.hunk = next;
    this.renderChrome(next);
    this.placeZone(next);
    const model = this.editor.getModel();
    if (model) this.bindModels(next, model, false);
    if (moved) this.position();
  }

  close() {
    if (!this.open && this.zoneId == null) {
      this.panel.style.visibility = "hidden";
      return;
    }
    this.open = false;
    this.hunk = null;
    this.revealHunk = null;
    this.zoneAfter = -1;
    this.zoneHeight = 0;
    this.panel.style.visibility = "hidden";
    this.diffEditor.setModel(null);
    if (this.zoneId) {
      const id = this.zoneId;
      this.zoneId = null;
      this.editor.changeViewZones((accessor) => accessor.removeZone(id));
    }
  }

  dispose() {
    this.close();
    for (const sub of this.subscriptions) sub.dispose();
    this.subscriptions.length = 0;
    this.diffEditor.dispose();
    this.panel.remove();
  }

  private step(delta: number) {
    if (!this.hunk) return;
    const list = hunksForBaseline(this.hunks(), this.hunk.baseline);
    if (list.length === 0) return;
    const index = list.findIndex((hunk) => sameHunk(hunk, this.hunk!));
    const next = list[(index + delta + list.length) % list.length];
    this.show(next, modifiedSpan(next).start, true);
  }

  private renderChrome(hunk: DiffHunk) {
    const list = hunksForBaseline(this.hunks(), hunk.baseline);
    const index = Math.max(0, list.findIndex((item) => sameHunk(item, hunk)));
    this.title.textContent = `${baselineLabel(hunk.baseline)} ${index + 1} / ${list.length}`;
    this.panel.style.borderColor = kindColor(hunk.kind);
  }

  private placeZone(hunk: DiffHunk) {
    const after = peekAfterLine(hunk);
    const lineHeight = this.editor.getOption(this.monaco.editor.EditorOption.lineHeight);
    const height = peekHeightPx(hunk, lineHeight, this.editor.getLayoutInfo().height);
    if (this.zoneId && this.zoneAfter === after && this.zoneHeight === height) return;
    this.zoneAfter = after;
    this.zoneHeight = height;
    this.editor.changeViewZones((accessor) => {
      if (this.zoneId) accessor.removeZone(this.zoneId);
      this.zoneId = accessor.addZone({
        afterLineNumber: after,
        heightInPx: height,
        domNode: this.zoneSpacer,
        suppressMouseDown: true,
        onDomNodeTop: () => this.position(),
        onComputedHeight: () => this.position(),
      });
    });
  }

  private bindModels(hunk: DiffHunk, modified: editor.ITextModel, reveal: boolean) {
    const filePath = this.getPath();
    const lookup = hunk.baseline === "worktree" ? filePath : headPathFor(filePath);
    const original = this.monaco.editor.getModel(
      blobUri(this.monaco, hunk.baseline === "worktree" ? "index" : "head", lookup),
    );
    if (!original) return;
    const current = this.diffEditor.getModel();
    const same = current?.original === original && current.modified === modified;
    if (reveal) this.revealHunk = hunk;
    if (!same) this.diffEditor.setModel({ original, modified });
    else if (reveal) {
      const line = Math.max(1, modifiedSpan(hunk).start);
      this.diffEditor.getModifiedEditor().revealLineInCenter(line);
      this.revealHunk = null;
    }
  }

  private position() {
    if (!this.open) return;
    const editorNode = this.editor.getDomNode();
    const editorRect = editorNode?.getBoundingClientRect();
    const zoneRect = this.zoneSpacer.getBoundingClientRect();
    if (
      !editorRect ||
      editorRect.width < 2 ||
      editorRect.height < 2 ||
      zoneRect.height < 2
    ) {
      this.panel.style.visibility = "hidden";
      return;
    }
    const top = Math.max(zoneRect.top, editorRect.top);
    const bottom = Math.min(zoneRect.bottom, editorRect.bottom);
    const left = Math.max(zoneRect.left, editorRect.left);
    const right = Math.min(zoneRect.right, editorRect.right);
    if (bottom - top < PEEK_HEADER_PX || right - left < 40) {
      this.panel.style.visibility = "hidden";
      return;
    }
    this.panel.style.visibility = "visible";
    this.panel.style.top = `${top}px`;
    this.panel.style.left = `${left}px`;
    this.panel.style.width = `${right - left}px`;
    this.panel.style.height = `${bottom - top}px`;
    this.diffEditor.layout({
      width: Math.floor(right - left),
      height: Math.max(0, Math.floor(bottom - top - PEEK_HEADER_PX)),
    });
  }

  private button(label: string, onClick: () => void): HTMLButtonElement {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "git-quick-diff-peek-btn";
    button.textContent = label;
    button.addEventListener("click", (event) => {
      event.preventDefault();
      event.stopPropagation();
      onClick();
    });
    return button;
  }
}

function sameHunk(a: DiffHunk, b: DiffHunk): boolean {
  return (
    a.baseline === b.baseline &&
    a.kind === b.kind &&
    a.originalStartLineNumber === b.originalStartLineNumber &&
    a.originalEndLineNumber === b.originalEndLineNumber &&
    a.modifiedStartLineNumber === b.modifiedStartLineNumber &&
    a.modifiedEndLineNumber === b.modifiedEndLineNumber
  );
}

function headPathFor(filePath: string): string {
  const staged = useGitStore
    .getState()
    .status.staged.find((file) => file.path === filePath);
  return staged ? stagedLookupPath(staged) : filePath;
}

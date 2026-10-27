import i18n from "../i18n";
/** Line-level git diff used by the editor gutter and the inline peek. */

export const MAX_QUICK_DIFF_BYTES = 1_000_000;
export const PEEK_HEADER_PX = 28;
export const PEEK_CONTEXT_LINES = 3;

export type DiffKind = "add" | "modify" | "delete";
export type DiffBaseline = "worktree" | "staged";

export interface LineChange {
  originalStartLineNumber: number;
  originalEndLineNumber: number;
  modifiedStartLineNumber: number;
  modifiedEndLineNumber: number;
}

export interface QuickDiffFile {
  path: string;
  status: string;
  orig_path?: string | null;
  untracked: boolean;
}

export interface DiffHunk {
  baseline: DiffBaseline;
  kind: DiffKind;
  originalStartLineNumber: number;
  originalEndLineNumber: number;
  modifiedStartLineNumber: number;
  modifiedEndLineNumber: number;
}

export interface DecorationRange {
  startLineNumber: number;
  startColumn: number;
  endLineNumber: number;
  endColumn: number;
}

export function exceedsQuickDiffLimit(bytes: number): boolean {
  return bytes > MAX_QUICK_DIFF_BYTES;
}

export function classifyChange(change: LineChange): DiffKind {
  if (change.originalEndLineNumber === 0) return "add";
  if (change.modifiedEndLineNumber === 0) return "delete";
  return "modify";
}

export function hunksFromChanges(
  baseline: DiffBaseline,
  changes: readonly LineChange[],
): DiffHunk[] {
  return changes.map((change) => ({
    baseline,
    kind: classifyChange(change),
    originalStartLineNumber: change.originalStartLineNumber,
    originalEndLineNumber: change.originalEndLineNumber,
    modifiedStartLineNumber: change.modifiedStartLineNumber,
    modifiedEndLineNumber: change.modifiedEndLineNumber,
  }));
}

/** Lines the gutter paints for this hunk. Deletes occupy the anchor line. */
export function modifiedSpan(hunk: DiffHunk): { start: number; end: number } {
  if (hunk.kind === "delete") {
    const line =
      hunk.modifiedStartLineNumber === 0 ? 1 : hunk.modifiedStartLineNumber;
    return { start: line, end: line };
  }
  const start = Math.max(1, hunk.modifiedStartLineNumber);
  const end = Math.max(start, hunk.modifiedEndLineNumber || start);
  return { start, end };
}

export function spansIntersect(
  a: { start: number; end: number },
  b: { start: number; end: number },
): boolean {
  return a.start <= b.end && b.start <= a.end;
}

/** Drop staged hunks whose lines meet a worktree hunk. */
export function dropOverlappingSecondary(hunks: readonly DiffHunk[]): DiffHunk[] {
  const primary = hunks.filter((hunk) => hunk.baseline === "worktree");
  const out: DiffHunk[] = [];
  for (const hunk of hunks) {
    if (hunk.baseline === "staged") {
      const span = modifiedSpan(hunk);
      if (primary.some((other) => spansIntersect(span, modifiedSpan(other)))) {
        continue;
      }
    }
    out.push(hunk);
  }
  return out;
}

export function decorationRange(hunk: DiffHunk, lineCount: number): DecorationRange {
  const span = modifiedSpan(hunk);
  const last = Math.max(1, lineCount);
  const start = Math.min(span.start, last);
  const end = Math.min(span.end, last);
  if (hunk.kind === "delete") {
    return {
      startLineNumber: start,
      startColumn: Number.MAX_SAFE_INTEGER,
      endLineNumber: start,
      endColumn: Number.MAX_SAFE_INTEGER,
    };
  }
  return {
    startLineNumber: start,
    startColumn: 1,
    endLineNumber: end,
    endColumn: 1,
  };
}

export function glyphClassName(hunk: DiffHunk): string {
  const kind =
    hunk.kind === "add"
      ? "dirty-diff-added"
      : hunk.kind === "delete"
        ? "dirty-diff-deleted"
        : "dirty-diff-modified";
  const tone = hunk.baseline === "worktree" ? "primary" : "secondary";
  return `dirty-diff-glyph ${kind} ${tone}`;
}

/** Click target: a line sits on the hunk, including a deletion at the top of the file. */
export function lineIntersectsHunk(lineNumber: number, hunk: DiffHunk): boolean {
  if (
    lineNumber === 1 &&
    hunk.modifiedStartLineNumber === 0 &&
    hunk.modifiedEndLineNumber === 0
  ) {
    return true;
  }
  const span = modifiedSpan(hunk);
  return lineNumber >= span.start && lineNumber <= span.end;
}

/** Worktree hunk wins when both baselines cover the line. */
export function hunkAtLine(hunks: readonly DiffHunk[], line: number): DiffHunk | null {
  const worktree = hunks.find(
    (hunk) => hunk.baseline === "worktree" && lineIntersectsHunk(line, hunk),
  );
  if (worktree) return worktree;
  return (
    hunks.find(
      (hunk) => hunk.baseline === "staged" && lineIntersectsHunk(line, hunk),
    ) ?? null
  );
}

export function hunksForBaseline(
  hunks: readonly DiffHunk[],
  baseline: DiffBaseline,
): DiffHunk[] {
  return hunks.filter((hunk) => hunk.baseline === baseline);
}

export function changeHeight(hunk: DiffHunk): number {
  const modified =
    hunk.modifiedEndLineNumber === 0
      ? 0
      : hunk.modifiedEndLineNumber - hunk.modifiedStartLineNumber + 1;
  const original =
    hunk.originalEndLineNumber === 0
      ? 0
      : hunk.originalEndLineNumber - hunk.originalStartLineNumber + 1;
  if (hunk.kind === "add") return Math.max(1, modified);
  if (hunk.kind === "delete") return Math.max(1, original);
  return Math.max(1, modified + original);
}

/** Line after which the peek zone opens. */
export function peekAfterLine(hunk: DiffHunk): number {
  if (hunk.modifiedEndLineNumber === 0) {
    return hunk.modifiedStartLineNumber === 0 ? 1 : hunk.modifiedStartLineNumber;
  }
  return hunk.modifiedEndLineNumber;
}

export function peekHeightPx(
  hunk: DiffHunk,
  lineHeight: number,
  editorHeight: number,
): number {
  const wanted = PEEK_HEADER_PX + (changeHeight(hunk) + PEEK_CONTEXT_LINES) * lineHeight;
  const floor = PEEK_HEADER_PX + lineHeight;
  const cap = Math.max(floor, Math.floor(editorHeight / 3));
  return Math.min(Math.max(wanted, floor), cap);
}

export function baselineLabel(baseline: DiffBaseline): string {
  return baseline === "worktree" ? i18n.t("git.worktree") : i18n.t("git.staged");
}

export interface QuickDiffSources {
  worktree: QuickDiffFile | null;
  staged: QuickDiffFile | null;
}

/** Which originals to load. Conflicts and untracked files draw nothing. */
export function originalsForPath(
  status: { is_repo: boolean; staged: readonly QuickDiffFile[]; changes: readonly QuickDiffFile[] },
  path: string,
): QuickDiffSources {
  if (!status.is_repo) return { worktree: null, staged: null };
  const worktree = status.changes.find((file) => file.path === path) ?? null;
  const staged = status.staged.find((file) => file.path === path) ?? null;
  if (worktree?.status === "U" || staged?.status === "U") {
    return { worktree: null, staged: null };
  }
  return {
    worktree: worktree && !worktree.untracked ? worktree : null,
    staged,
  };
}

/** HEAD text for a staged file. A brand-new staged file diffs against empty. */
export function stagedOriginalText(file: QuickDiffFile, head: string | null): string | null {
  if (head !== null) return head;
  if (file.status === "A") return "";
  return null;
}

export function stagedLookupPath(file: QuickDiffFile): string {
  return file.orig_path || file.path;
}

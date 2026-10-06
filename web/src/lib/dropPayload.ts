import { fileNameFromPath } from "../utils/language";

/** Workspace paths written by the file tree. */
export const LITECODE_PATHS_MIME = "application/x-litecode-paths";

/** Monaco selection: path plus 1-based line span. The plain-text payload stays the code. */
export const LITECODE_SPAN_MIME = "application/x-litecode-span";

export interface CodeSpan {
  path: string;
  start: number;
  end: number;
}

export interface LineSelection {
  isEmpty(): boolean;
  startLineNumber: number;
  endLineNumber: number;
  endColumn: number;
}

export function parsePathPayload(raw: string): string[] | null {
  const text = raw.trim();
  if (!text) return null;
  try {
    const parsed = JSON.parse(text) as unknown;
    if (Array.isArray(parsed) && parsed.every((entry) => typeof entry === "string")) {
      const paths = parsed.map((entry) => entry.trim()).filter(Boolean);
      return paths.length > 0 ? paths : null;
    }
  } catch {
    /* newline-separated paths */
  }
  const paths = text
    .split(/\r?\n/)
    .map((entry) => entry.trim())
    .filter(Boolean);
  return paths.length > 0 ? paths : null;
}

/** Paths the file tree put on the drag. Plain text is not consulted. */
export function readTreePaths(dt: DataTransfer): string[] | null {
  return parsePathPayload(dt.getData(LITECODE_PATHS_MIME));
}

export function readCodeSpan(dt: DataTransfer): CodeSpan | null {
  const raw = dt.getData(LITECODE_SPAN_MIME);
  if (!raw) return null;
  try {
    const parsed = JSON.parse(raw) as Partial<CodeSpan>;
    if (typeof parsed.path !== "string" || parsed.path.length === 0) return null;
    if (typeof parsed.start !== "number" || typeof parsed.end !== "number") return null;
    if (parsed.start < 1 || parsed.end < parsed.start) return null;
    return { path: parsed.path, start: parsed.start, end: parsed.end };
  } catch {
    return null;
  }
}

export function writeCodeSpan(dt: DataTransfer, span: CodeSpan): void {
  dt.setData(LITECODE_SPAN_MIME, JSON.stringify(span));
}

/** Same end-line trim as "Add to chat": a caret sitting on the next line is not part of the span. */
export function spanFromSelection(
  path: string,
  selection: LineSelection | null,
): CodeSpan | null {
  if (!path || !selection || selection.isEmpty()) return null;
  let end = selection.endLineNumber;
  if (selection.endColumn === 1 && end > selection.startLineNumber) end -= 1;
  if (end < selection.startLineNumber) return null;
  return { path, start: selection.startLineNumber, end };
}

function decodeFileUri(raw: string): string | null {
  const trimmed = raw.trim();
  if (!/^file:/i.test(trimmed)) return null;
  try {
    const url = new URL(trimmed);
    let path = decodeURIComponent(url.pathname);
    if (/^\/[A-Za-z]:\//.test(path)) path = path.slice(1);
    return path;
  } catch {
    return null;
  }
}

function isAbsolutePathLine(line: string): boolean {
  if (/^file:/i.test(line)) return true;
  if (line.startsWith("\\\\") || line.startsWith("//")) return true;
  if (line.startsWith("/")) return true;
  return /^[A-Za-z]:[\\/]/.test(line);
}

/**
 * Path text that is not a file tree payload and not a code span.
 * `text/uri-list` wins; otherwise every plain-text line must be an absolute path.
 */
export function readLoosePaths(dt: DataTransfer): string[] {
  const uri = dt.getData("text/uri-list");
  if (uri) {
    const lines = uri
      .split(/\r?\n/)
      .map((entry) => entry.trim())
      .filter((entry) => entry.length > 0 && !entry.startsWith("#"));
    if (lines.length > 0) return lines;
  }
  const text = dt.getData("text/plain").trim();
  if (!text) return [];
  const lines = text
    .split(/\r?\n/)
    .map((entry) => entry.trim())
    .filter(Boolean);
  if (lines.length === 0 || !lines.every(isAbsolutePathLine)) return [];
  return lines;
}

/** Desktop host path, when the preload bridge can see the File. */
export function osFilePath(file: File): string | null {
  const fromHost = window.litecode?.getPathForFile?.(file);
  if (typeof fromHost === "string" && fromHost.length > 0) return fromHost;
  return null;
}

/**
 * Chip path. Inside `projectRoot` this is workspace-relative with forward
 * slashes. Otherwise the absolute path, also with forward slashes. A quote
 * would break the shortcode, so the file name (quotes stripped) is used instead.
 */
export function chipPath(raw: string, projectRoot: string): string {
  const decoded = decodeFileUri(raw) ?? raw.trim();
  const slash = decoded.replaceAll("\\", "/");
  if (!slash) return "file";
  if (slash.includes('"')) {
    const name = fileNameFromPath(slash).replaceAll('"', "");
    return name || "file";
  }
  const root = projectRoot.trim().replaceAll("\\", "/").replace(/\/+$/, "");
  if (root) {
    const pathKey = slash.toLowerCase();
    const rootKey = root.toLowerCase();
    if (pathKey === rootKey) return fileNameFromPath(slash) || "file";
    if (pathKey.startsWith(`${rootKey}/`)) return slash.slice(root.length + 1);
  }
  return slash;
}

export function dragCarriesMention(dt: DataTransfer): boolean {
  const types = new Set(Array.from(dt.types));
  if (types.has(LITECODE_PATHS_MIME) || types.has(LITECODE_SPAN_MIME)) return true;
  if (dt.files.length > 0 || types.has("Files")) return true;
  if (types.has("text/uri-list")) return true;
  return false;
}

/** Drop can also see a plain-text absolute path. Drag-over often cannot. */
export function dropCarriesMention(dt: DataTransfer): boolean {
  return dragCarriesMention(dt) || readLoosePaths(dt).length > 0;
}

/** Drag-over can see file and URI drags. Reading text/plain here is unreliable. */
export function dragOverClaimsForeign(dt: DataTransfer): boolean {
  const types = new Set(Array.from(dt.types));
  if (types.has(LITECODE_PATHS_MIME) || types.has(LITECODE_SPAN_MIME)) return false;
  if (dt.files.length > 0 || types.has("Files")) return true;
  if (types.has("text/uri-list")) return true;
  return false;
}

export function dropClaimsForeign(dt: DataTransfer): boolean {
  if (dragOverClaimsForeign(dt)) return true;
  const types = new Set(Array.from(dt.types));
  if (types.has(LITECODE_PATHS_MIME) || types.has(LITECODE_SPAN_MIME)) return false;
  return readLoosePaths(dt).length > 0;
}

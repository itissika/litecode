/** How the editor tab should present a workspace path. */
export type FileKind =
  | "text"
  | "image"
  | "pdf"
  | "audio"
  | "video"
  | "sqlite"
  | "binary";

/** Shown when the file is binary and Monaco must not receive the bytes. */
export const BINARY_FILE_MESSAGE = "Binary file. Can't display it here.";

const IMAGE_EXT = new Set([
  "png",
  "jpg",
  "jpeg",
  "gif",
  "webp",
  "bmp",
  "ico",
  "avif",
  "svg",
]);

const AUDIO_EXT = new Set(["mp3", "wav", "ogg", "oga", "m4a", "aac", "flac"]);

const VIDEO_EXT = new Set(["mp4", "webm"]);

const SQLITE_EXT = new Set(["db", "sqlite", "sqlite3"]);

const BINARY_EXT = new Set([
  "zip",
  "gz",
  "tgz",
  "bz2",
  "xz",
  "7z",
  "rar",
  "tar",
  "exe",
  "dll",
  "so",
  "dylib",
  "wasm",
  "bin",
  "class",
  "jar",
  "war",
  "ear",
  "iso",
  "dmg",
  "pdb",
  "obj",
  "o",
  "a",
  "lib",
  "pyc",
  "pyo",
  "woff",
  "woff2",
  "ttf",
  "otf",
  "eot",
  "doc",
  "docx",
  "xls",
  "xlsx",
  "ppt",
  "pptx",
  "odt",
  "ods",
  "odp",
  "psd",
  "ai",
  "sketch",
  "blend",
]);

export function extensionOf(path: string): string {
  const base = path.split(/[/\\]/).pop() ?? path;
  const dot = base.lastIndexOf(".");
  if (dot <= 0) return "";
  return base.slice(dot + 1).toLowerCase();
}

export function fileKindFromPath(path: string): FileKind {
  const ext = extensionOf(path);
  if (IMAGE_EXT.has(ext)) return "image";
  if (ext === "pdf") return "pdf";
  if (AUDIO_EXT.has(ext)) return "audio";
  if (VIDEO_EXT.has(ext)) return "video";
  if (SQLITE_EXT.has(ext)) return "sqlite";
  if (BINARY_EXT.has(ext)) return "binary";
  return "text";
}

export function isTextKind(kind: FileKind): boolean {
  return kind === "text";
}

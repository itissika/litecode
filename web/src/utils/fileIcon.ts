/**
 * File/folder icon resolution for the file tree, using the project's own
 * phosphor icon set so icons share the theme's color tokens (phosphor glyphs
 * use `currentColor`) and visual weight with the rest of the UI.
 *
 * Folders use a single unified icon (`FolderSimple`); only individual files
 * get a language/file-type specific glyph. The mapping is intentionally
 * coarse (one glyph per language family) — that is the tradeoff for theme
 * unity versus a multi-color, per-extension set like vscode-icons.
 */
import type { Icon } from "@phosphor-icons/react";
import {
  File,
  FileArchive,
  FileAudio,
  FileCode,
  FileCss,
  FileCsv,
  FileDoc,
  FileHtml,
  FileImage,
  FileJs,
  FileJsx,
  FileLock,
  FilePdf,
  FilePy,
  FileRs,
  FileSql,
  FileText,
  FileTs,
  FileTsx,
  FileVideo,
  FileVue,
  FileXls,
  FileZip,
  FolderSimple,
} from "@phosphor-icons/react";

const EXT_ICON: Record<string, Icon> = {
  // scripts / code
  ts: FileTs,
  tsx: FileTsx,
  js: FileJs,
  jsx: FileJsx,
  mjs: FileJs,
  cjs: FileJs,
  json: FileCode,
  jsonc: FileCode,
  xml: FileCode,
  rs: FileRs,
  py: FilePy,
  pyw: FilePy,
  pyi: FilePy,
  go: FileCode,
  java: FileCode,
  c: FileCode,
  h: FileCode,
  cpp: FileCode,
  hpp: FileCode,
  cc: FileCode,
  hh: FileCode,
  cs: FileCode,
  rb: FileCode,
  php: FileCode,
  swift: FileCode,
  kt: FileCode,
  kts: FileCode,
  lua: FileCode,
  dart: FileCode,
  ex: FileCode,
  exs: FileCode,
  sh: FileCode,
  bash: FileCode,
  zsh: FileCode,
  fish: FileCode,
  vue: FileVue,
  svelte: FileCode,
  // config / docs
  md: FileText,
  mdx: FileText,
  markdown: FileText,
  toml: FileText,
  yaml: FileText,
  yml: FileText,
  ini: FileText,
  cfg: FileText,
  conf: FileText,
  // styles
  css: FileCss,
  scss: FileCss,
  sass: FileCss,
  less: FileCss,
  html: FileHtml,
  htm: FileHtml,
  // data
  csv: FileCsv,
  tsv: FileCsv,
  sql: FileSql,
  // images
  png: FileImage,
  jpg: FileImage,
  jpeg: FileImage,
  gif: FileImage,
  ico: FileImage,
  webp: FileImage,
  bmp: FileImage,
  avif: FileImage,
  svg: FileImage,
  // media
  mp3: FileAudio,
  wav: FileAudio,
  ogg: FileAudio,
  flac: FileAudio,
  mp4: FileVideo,
  webm: FileVideo,
  mov: FileVideo,
  avi: FileVideo,
  mkv: FileVideo,
  // documents
  pdf: FilePdf,
  doc: FileDoc,
  docx: FileDoc,
  rtf: FileDoc,
  xls: FileXls,
  xlsx: FileXls,
  // archives
  zip: FileZip,
  gz: FileArchive,
  tar: FileArchive,
  rar: FileArchive,
  "7z": FileArchive,
  bz2: FileArchive,
  xz: FileArchive,
  // misc
  lock: FileLock,
};

// Filename-based overrides (no/ambiguous extension).
const NAME_ICON: Record<string, Icon> = {
  dockerfile: FileCode,
  makefile: FileCode,
  "cmakelists.txt": FileCode,
  license: FileText,
  "license.md": FileText,
  readme: FileText,
  "readme.md": FileText,
  ".gitignore": FileText,
  ".npmrc": FileCode,
  ".env": FileCode,
  ".editorconfig": FileCode,
};

/** Phosphor icon component for a file, chosen by name/extension. */
export function getFileIcon(name: string): Icon {
  const lower = name.toLowerCase();
  const named = NAME_ICON[lower];
  if (named) return named;
  const ext = lower.includes(".") ? (lower.split(".").pop() as string) : "";
  return EXT_ICON[ext] ?? File;
}

/** Single unified folder icon (matches the file-tree panel icon). */
export const FolderIcon: Icon = FolderSimple;

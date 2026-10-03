export type KnowledgeSegment =
  | { type: "text"; value: string }
  | { type: "ref"; id: string; label: string };

/**
 * Declaration keys may include letters, numbers, `_`, `-`, and single spaces:
 * `seq`, `knowledge 概念概述`. Quotes and brackets stay out so the key can be
 * a path segment and a Mention shortcode attribute.
 */
export const KNOWLEDGE_KEY = /^[\p{L}\p{N}_][\p{L}\p{N}_-]*(?: [\p{L}\p{N}_-]+)*$/u;

/** Node citation. Only `key` is recognized. */
const SHORTCODE_SOURCE = String.raw`\[@ key="([^"]*)"\]`;

/**
 * File, symbol, or line-range citation.
 * Attribute order is `file`, optional `symbol`, optional `lines`.
 */
const FILE_ATTR_SOURCE = String.raw`\[@ file="([^"]*)"(?: symbol="([^"]*)")?(?: lines="([^"]*)")?\]`;

/** Compare and look up keys after trimming. The stored key is left unchanged. */
export function normalizeKey(key: string): string {
  return key.trim();
}

export function isKnowledgeKey(key: string): boolean {
  return KNOWLEDGE_KEY.test(key);
}

/** One node citation. A second argument is ignored; display text is not stored. */
export function mentionSource(id: string, _label = id): string {
  return `[@ key="${id}"]`;
}

/** Words shown for a file or symbol citation. A node shows its key. */
export function citationFact(
  path: string,
  symbol?: string | null,
  lines?: string | null,
): string {
  let out = path;
  const chain = symbol?.trim() ?? "";
  if (chain) out += ` : ${chain}`;
  const raw = lines?.trim() ?? "";
  const span = raw ? parseLineSpan(raw) : null;
  if (span) out += ` : ${formatLineSpan(span.start, span.end)}`;
  return out;
}

/** Visible text for a workspace path: the last segment. */
export function fileLabel(path: string): string {
  const parts = pathParts(path);
  return parts[parts.length - 1] || path;
}

/** Blue chip: `.../parent/file`, or just the file when it has no parent. */
export function humanFileLabel(path: string): string {
  const parts = pathParts(path);
  const file = parts[parts.length - 1] || path;
  const parent = parts.length >= 2 ? parts[parts.length - 2] : "";
  if (!parent) return file;
  return `.../${parent}/${file}`;
}

/** Cyan chip: `file : symbol`. Lines stay off the capsule. */
export function humanSymbolLabel(path: string, symbol: string): string {
  return `${fileLabel(path)} : ${symbol.trim()}`;
}

function pathParts(path: string): string[] {
  return path.replaceAll("\\", "/").split("/").filter((part) => part.length > 0);
}

/** One file citation. A label argument is ignored. */
export function fileMentionSource(path: string, _label = fileLabel(path)): string {
  return `[@ file="${path}"]`;
}

/** `12` or `2148-2165`. Zero and an inverted range are not spans. */
export function parseLineSpan(raw: string): { start: number; end: number } | null {
  const text = raw.trim();
  const match = /^(\d+)(?:-(\d+))?$/.exec(text);
  if (!match) return null;
  const start = Number(match[1]);
  const end = match[2] ? Number(match[2]) : start;
  if (!start || !end || end < start) return null;
  return { start, end };
}

export function formatLineSpan(start: number, end: number): string {
  return start === end ? String(start) : `${start}-${end}`;
}

/**
 * One symbol or range mention as TipTap writes it.
 * Omit `symbol` for a line range. Omit `lines` for a knowledge-body citation.
 */
export function symbolMentionSource(
  path: string,
  options: { symbol?: string; lines?: string; label?: string } = {},
): string {
  const symbol = options.symbol?.trim() ?? "";
  const lines = options.lines?.trim() ?? "";
  let out = `[@ file="${path}"`;
  if (symbol) out += ` symbol="${symbol}"`;
  if (lines) out += ` lines="${lines}"`;
  out += "]";
  return out;
}

/**
 * A workspace-relative path. `..`, `.`, an empty segment, and an absolute path
 * (leading slash or a drive letter) are not paths this library can check.
 */
export function isWorkspaceFileRef(path: string): boolean {
  const slash = path.replaceAll("\\", "/");
  if (!slash || Array.from(slash).length > 512) return false;
  if (slash.startsWith("/")) return false;
  if (slash.charAt(1) === ":") return false;
  const parts = slash.split("/");
  return parts.every((part) => part.length > 0 && part !== "." && part !== "..");
}

function shortcodePattern(): RegExp {
  return new RegExp(SHORTCODE_SOURCE, "g");
}

function fileAttrPattern(): RegExp {
  return new RegExp(FILE_ATTR_SOURCE, "g");
}

/**
 * Split one text run into literal pieces and Mention shortcodes.
 * Plain `@seq` and any other bracket form stay literal text.
 */
export function splitKnowledgeRefs(text: string): KnowledgeSegment[] {
  const out: KnowledgeSegment[] = [];
  let last = 0;
  for (const match of text.matchAll(shortcodePattern())) {
    const id = normalizeKey(match[1] ?? "");
    const start = match.index ?? 0;
    if (!id || !isKnowledgeKey(id)) continue;
    if (start > last) out.push({ type: "text", value: text.slice(last, start) });
    out.push({ type: "ref", id, label: id });
    last = start + match[0].length;
  }
  if (last < text.length) out.push({ type: "text", value: text.slice(last) });
  if (out.length === 0) out.push({ type: "text", value: text });
  return out;
}

/** Replace an inline code span with spaces so shortcodes inside it disappear. */
function maskInlineCode(line: string): string {
  let out = "";
  let i = 0;
  while (i < line.length) {
    if (line[i] !== "`") {
      out += line[i];
      i += 1;
      continue;
    }
    let ticks = 0;
    while (line[i + ticks] === "`") ticks += 1;
    const closer = "`".repeat(ticks);
    const closeAt = line.indexOf(closer, i + ticks);
    if (closeAt === -1) {
      out += line.slice(i);
      break;
    }
    out += " ".repeat(closeAt + ticks - i);
    i = closeAt + ticks;
  }
  return out;
}

interface FenceScan {
  /** Prose lines, with inline code masked out. Fence lines are omitted. */
  prose: string[];
}

/** Walk markdown line by line, dropping fenced blocks (``` and ~~~). */
function scanProse(markdown: string): FenceScan {
  const prose: string[] = [];
  let fenceChar = "";
  let fenceLen = 0;
  for (const line of markdown.split("\n")) {
    if (fenceLen > 0) {
      const closed = new RegExp(
        `^ {0,3}${fenceChar}{${fenceLen},}\\s*$`,
      ).test(line);
      if (closed) fenceLen = 0;
      continue;
    }
    const open = /^ {0,3}(`{3,}|~{3,})/.exec(line);
    if (open) {
      fenceChar = open[1]![0]!;
      fenceLen = open[1]!.length;
      continue;
    }
    prose.push(maskInlineCode(line));
  }
  return { prose };
}

export type BodySegment =
  | { type: "text"; value: string }
  | { type: "ref"; id: string; label: string }
  | { type: "file"; path: string; label: string }
  | {
      type: "symbol";
      path: string;
      symbol: string | null;
      lines: string | null;
      label: string;
    };

/** Split one text run into prose, node mentions, and file mentions, in order. */
export function splitBodyRefs(text: string): BodySegment[] {
  const hits: Array<{
    start: number;
    end: number;
    kind: "ref" | "file" | "symbol";
    a: string;
    b: string;
    symbol?: string | null;
    lines?: string | null;
  }> = [];
  for (const match of text.matchAll(shortcodePattern())) {
    const id = normalizeKey(match[1] ?? "");
    if (!id || !isKnowledgeKey(id)) continue;
    const start = match.index ?? 0;
    hits.push({
      start,
      end: start + match[0].length,
      kind: "ref",
      a: id,
      b: id,
    });
  }
  for (const match of text.matchAll(fileAttrPattern())) {
    const path = (match[1] ?? "").trim();
    if (!path) continue;
    const symbol = (match[2] ?? "").trim();
    const lines = (match[3] ?? "").trim();
    const parsedLines = lines ? parseLineSpan(lines) : null;
    const start = match.index ?? 0;
    const isSymbol = Boolean(symbol) || parsedLines !== null;
    const label = citationFact(path, symbol, parsedLines ? lines : null);
    hits.push({
      start,
      end: start + match[0].length,
      kind: isSymbol ? "symbol" : "file",
      a: path,
      b: label,
      symbol: symbol || null,
      lines: parsedLines ? lines : null,
    });
  }
  hits.sort((left, right) => left.start - right.start || left.end - right.end);
  const out: BodySegment[] = [];
  let last = 0;
  for (const hit of hits) {
    if (hit.start < last) continue;
    if (hit.start > last) out.push({ type: "text", value: text.slice(last, hit.start) });
    if (hit.kind === "ref") out.push({ type: "ref", id: hit.a, label: hit.b });
    else if (hit.kind === "symbol") {
      out.push({
        type: "symbol",
        path: hit.a,
        symbol: hit.symbol ?? null,
        lines: hit.lines ?? null,
        label: hit.b,
      });
    } else out.push({ type: "file", path: hit.a, label: hit.b });
    last = hit.end;
  }
  if (last < text.length) out.push({ type: "text", value: text.slice(last) });
  if (out.length === 0) out.push({ type: "text", value: text });
  return out;
}

export interface CitedSymbol {
  path: string;
  symbol: string;
}

/** Symbol citations in prose, first chain per file wins. Range-only citations are not included. */
export function extractSymbolRefs(markdown: string): CitedSymbol[] {
  const out: CitedSymbol[] = [];
  const seen = new Set<string>();
  for (const line of scanProse(markdown).prose) {
    for (const segment of splitBodyRefs(line)) {
      if (segment.type !== "symbol" || !segment.symbol) continue;
      const key = `${segment.path}\0${segment.symbol}`;
      if (seen.has(key)) continue;
      seen.add(key);
      out.push({ path: segment.path, symbol: segment.symbol });
    }
  }
  return out;
}

/** File paths cited in prose. Fenced blocks and inline code are not citations. */
export function extractFileRefs(markdown: string): string[] {
  const paths: string[] = [];
  const seen = new Set<string>();
  for (const line of scanProse(markdown).prose) {
    for (const match of line.matchAll(fileAttrPattern())) {
      const path = (match[1] ?? "").trim();
      if (!path || seen.has(path)) continue;
      seen.add(path);
      paths.push(path);
    }
  }
  return paths;
}

export interface KnowledgeMention {
  id: string;
  label: string;
}

/** Mentions in prose, first id wins. Fenced blocks and inline code are not mentions. */
export function extractMentions(markdown: string): KnowledgeMention[] {
  const mentions: KnowledgeMention[] = [];
  const seen = new Set<string>();
  for (const line of scanProse(markdown).prose) {
    for (const segment of splitKnowledgeRefs(line)) {
      if (segment.type !== "ref" || seen.has(segment.id)) continue;
      seen.add(segment.id);
      mentions.push({ id: segment.id, label: segment.label });
    }
  }
  return mentions;
}

/** Ids cited in prose, first-seen order. */
export function extractMarkers(markdown: string): string[] {
  return extractMentions(markdown).map((mention) => mention.id);
}

/**
 * Rewrite mention ids that equal `from`. A label equal to the old id is
 * rewritten too. The `node :` declaration is not a shortcode, so it stays.
 */
export function replaceMentionKey(text: string, from: string, to: string): string {
  const source = normalizeKey(from);
  const target = normalizeKey(to);
  if (!source || source === target) return text;
  return text.replace(shortcodePattern(), (full, id: string) => {
    const idKey = normalizeKey(id);
    if (!isKnowledgeKey(idKey) || idKey !== source) return full;
    return mentionSource(target);
  });
}

/** First non-empty prose line as literal/ref segments (for one-line summaries). */
export function knowledgeFirstLineSegments(value: string): KnowledgeSegment[] {
  for (const line of scanProse(value).prose) {
    if (!line.trim()) continue;
    return splitKnowledgeRefs(line);
  }
  return [{ type: "text", value: "" }];
}

const PREVIEW_CODE_CHARS = 24;

function clipChars(value: string, max: number): string {
  const chars = Array.from(value);
  if (chars.length <= max) return value;
  return `${chars.slice(0, max).join("")}…`;
}

function replaceShortcodesWithLabels(text: string): string {
  const nodes = text.replace(shortcodePattern(), (full, id: string) => {
    const key = normalizeKey(id);
    if (!isKnowledgeKey(key)) return full;
    return key;
  });
  return nodes.replace(
    fileAttrPattern(),
    (full, path: string, symbol: string, lines: string) => {
      const cleaned = path.trim();
      if (!cleaned) return full;
      const chain = symbol?.trim() ?? "";
      const raw = lines?.trim() ?? "";
      return citationFact(cleaned, chain, raw ? raw : null);
    },
  );
}

/** Show inline code, clipped. Shortcodes are replaced only outside code spans. */
function showInlineCode(line: string): string {
  let out = "";
  let text = "";
  let i = 0;
  const flush = () => {
    if (!text) return;
    out += replaceShortcodesWithLabels(text);
    text = "";
  };
  while (i < line.length) {
    if (line[i] !== "`") {
      text += line[i];
      i += 1;
      continue;
    }
    let ticks = 0;
    while (line[i + ticks] === "`") ticks += 1;
    const closer = "`".repeat(ticks);
    const closeAt = line.indexOf(closer, i + ticks);
    if (closeAt === -1) {
      text += line.slice(i);
      break;
    }
    flush();
    out += `\`${clipChars(line.slice(i + ticks, closeAt), PREVIEW_CODE_CHARS)}\``;
    i = closeAt + ticks;
  }
  flush();
  return out;
}

function previewLines(markdown: string): string[] {
  const prose: string[] = [];
  let fenceChar = "";
  let fenceLen = 0;
  for (const line of markdown.split("\n")) {
    if (fenceLen > 0) {
      const closed = new RegExp(
        `^ {0,3}${fenceChar}{${fenceLen},}\\s*$`,
      ).test(line);
      if (closed) fenceLen = 0;
      continue;
    }
    const open = /^ {0,3}(`{3,}|~{3,})/.exec(line);
    if (open) {
      fenceChar = open[1]![0]!;
      fenceLen = open[1]!.length;
      continue;
    }
    prose.push(showInlineCode(line));
  }
  return prose;
}

/** First lines of a value, with mentions reduced to their label, for card previews. */
export function knowledgePreview(value: string, lines = 3): string {
  return previewLines(value)
    .map((line) => line.trim())
    .filter((line) => line.length > 0)
    .slice(0, lines)
    .join("\n");
}

type KnowledgeSegment =
  | { type: "text"; value: string }
  | { type: "ref"; id: string; label: string };

/**
 * Declaration keys may include letters, numbers, `_`, `-`, and single spaces:
 * `seq`, `knowledge 概念概述`. Quotes and brackets stay out so the key can be
 * a path segment and a Mention shortcode attribute.
 */
const KNOWLEDGE_KEY = /^[\p{L}\p{N}_][\p{L}\p{N}_-]*(?: [\p{L}\p{N}_-]+)*$/u;

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

/**
 * Words shown for a file, symbol, or range citation. A node shows its key.
 * A symbol citation never shows lines: symbol and line-range citations do not mix.
 */
export function citationFact(
  path: string,
  symbol?: string | null,
  lines?: string | null,
): string {
  let out = path;
  const chain = symbol?.trim() ?? "";
  if (chain) return `${out} : ${chain}`;
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

const MAX_LINE = 4294967295;

/**
 * `12` or `2148-2165` (KR-SYN-5): ASCII digits, no leading zero, no sign, no
 * spaces, at most u32::MAX, start <= end. Anything else is not a span.
 */
export function parseLineSpan(raw: string): { start: number; end: number } | null {
  const match = /^([1-9][0-9]*)(?:-([1-9][0-9]*))?$/.exec(raw);
  if (!match) return null;
  const start = Number(match[1]);
  const end = match[2] ? Number(match[2]) : start;
  if (start > MAX_LINE || end > MAX_LINE || end < start) return null;
  return { start, end };
}

export function formatLineSpan(start: number, end: number): string {
  return start === end ? String(start) : `${start}-${end}`;
}

/**
 * One symbol or range mention as TipTap writes it.
 * Omit `symbol` for a line range. Omit `lines` for a knowledge-body citation.
 * A symbol citation never gets `lines` (KR-SYM-6): new insertions drop them.
 * `asWritten` re-emits existing text unchanged (an editor round-trip), so a
 * symbol written with lines keeps them and the backend keeps reporting
 * `invalid_lines` instead of the frontend silently rewriting the file.
 */
export function symbolMentionSource(
  path: string,
  options: { symbol?: string; lines?: string; label?: string; asWritten?: boolean } = {},
): string {
  const symbol = options.symbol?.trim() ?? "";
  const lines = options.asWritten ? (options.lines ?? "") : (options.lines?.trim() ?? "");
  let out = `[@ file="${path}"`;
  if (symbol) out += ` symbol="${symbol}"`;
  if (lines && (!symbol || options.asWritten)) out += ` lines="${lines}"`;
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
  | {
      type: "file";
      path: string;
      label: string;
      /** `lines` attribute exactly as written (a malformed span), or null. */
      rawLines?: string | null;
    }
  | {
      type: "symbol";
      path: string;
      /** Chain for a symbol citation; null for a line range. */
      symbol: string | null;
      /** Valid span of a line range. Always null on a symbol citation. */
      lines: string | null;
      label: string;
      /** `lines` attribute exactly as written, or null. */
      rawLines?: string | null;
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
    rawLines?: string | null;
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
    const rawLines = match[3] ?? null;
    const lines = (rawLines ?? "").trim();
    // A symbol citation takes no lines; only a line range carries a span.
    const parsedLines = !symbol && lines ? parseLineSpan(lines) : null;
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
      rawLines: rawLines === "" ? null : rawLines,
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
        ...(hit.rawLines != null ? { rawLines: hit.rawLines } : {}),
      });
    } else {
      out.push({
        type: "file",
        path: hit.a,
        label: hit.b,
        ...(hit.rawLines != null ? { rawLines: hit.rawLines } : {}),
      });
    }
    last = hit.end;
  }
  if (last < text.length) out.push({ type: "text", value: text.slice(last) });
  if (out.length === 0) out.push({ type: "text", value: text });
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

/** Path part of a `ref_key` (KR-KEY-1): trimmed, `\\` as `/`, nothing else. */
function normalizeRefPath(path: string): string {
  return path.trim().replaceAll("\\", "/");
}

/** One citation as the reference identity needs it. `lines` is the raw attribute. */
type RefKeyInput =
  | { kind: "node"; key: string }
  | { kind: "file"; path: string; symbol?: string | null; lines?: string | null };

/**
 * `ref_key` (KR-KEY-1), the one identity frontend, backend, and contract table share:
 * `node:<key>`, `file:<path>`, `sym:<path>\0<chain>`, `range:<path>\0<lines>`.
 * A symbol key never includes lines. A valid span is normalized (`5-5` → `5`);
 * a malformed one stays as written.
 */
export function knowledgeRefKey(ref: RefKeyInput): string {
  if (ref.kind === "node") return `node:${normalizeKey(ref.key)}`;
  const path = normalizeRefPath(ref.path);
  const chain = ref.symbol?.trim() ?? "";
  if (chain) return `sym:${path}\0${chain}`;
  const raw = ref.lines ?? "";
  if (raw.length > 0) {
    const span = parseLineSpan(raw);
    const lines = span ? formatLineSpan(span.start, span.end) : raw;
    return `range:${path}\0${lines}`;
  }
  return `file:${path}`;
}

interface ExtractedRef {
  kind: "node" | "file" | "symbol" | "range";
  key?: string;
  path?: string;
  symbol?: string;
  /** Normalized span of a valid range. Null on a symbol and on a malformed range. */
  lines?: string | null;
  /** Malformed `lines` text as written; on a symbol, any `lines` at all (KR-SYM-6). */
  bad_lines?: string;
  ref_key: string;
}

/**
 * Every citation in prose, in document order, repeats kept (KR-SYN-8).
 * Fenced blocks and inline code are not citations. Shapes match
 * `cases.json` `extract[].refs`.
 */
export function extractRefs(markdown: string): ExtractedRef[] {
  const out: ExtractedRef[] = [];
  for (const line of scanProse(markdown).prose) {
    const hits: Array<{ start: number; ref: ExtractedRef }> = [];
    for (const match of line.matchAll(shortcodePattern())) {
      const key = normalizeKey(match[1] ?? "");
      if (!key || !isKnowledgeKey(key)) continue;
      hits.push({
        start: match.index ?? 0,
        ref: { kind: "node", key, ref_key: knowledgeRefKey({ kind: "node", key }) },
      });
    }
    for (const match of line.matchAll(fileAttrPattern())) {
      const path = normalizeRefPath(match[1] ?? "");
      if (!path) continue;
      const symbol = (match[2] ?? "").trim();
      const rawLines = match[3] ?? "";
      const ref_key = knowledgeRefKey({ kind: "file", path, symbol, lines: rawLines });
      let ref: ExtractedRef;
      if (symbol) {
        // KR-SYM-6: lines on a symbol are a format error whatever they say.
        ref = rawLines
          ? { kind: "symbol", path, symbol, lines: null, bad_lines: rawLines, ref_key }
          : { kind: "symbol", path, symbol, lines: null, ref_key };
      } else if (rawLines) {
        const span = parseLineSpan(rawLines);
        ref = span
          ? { kind: "range", path, lines: formatLineSpan(span.start, span.end), ref_key }
          : { kind: "range", path, lines: null, bad_lines: rawLines, ref_key };
      } else {
        ref = { kind: "file", path, ref_key };
      }
      hits.push({ start: match.index ?? 0, ref });
    }
    hits.sort((left, right) => left.start - right.start);
    for (const hit of hits) out.push(hit.ref);
  }
  return out;
}

interface KnowledgeMention {
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

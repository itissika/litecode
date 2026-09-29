export type KnowledgeSegment =
  | { type: "text"; value: string }
  | { type: "ref"; id: string; label: string };

/**
 * Declaration keys may include letters, numbers, `_`, `-`, and single spaces:
 * `seq`, `knowledge 概念概述`. Quotes and brackets stay out so the key can be
 * a path segment and a Mention shortcode attribute.
 */
export const KNOWLEDGE_KEY = /^[\p{L}\p{N}_][\p{L}\p{N}_-]*(?: [\p{L}\p{N}_-]+)*$/u;

/** TipTap Mention markdown: `[@ id="seq" label="seq"]`. `id` then `label`, double quotes. */
const SHORTCODE_SOURCE = String.raw`\[@ id="([^"]*)" label="([^"]*)"\]`;

/** Workspace path mention: `[@ file="src/a.rs" label="a.rs"]`. Not a node citation. */
const FILE_SHORTCODE_SOURCE = String.raw`\[@ file="([^"]*)" label="([^"]*)"\]`;

/** Compare and look up keys after trimming. The stored key is left unchanged. */
export function normalizeKey(key: string): string {
  return key.trim();
}

export function isKnowledgeKey(key: string): boolean {
  return KNOWLEDGE_KEY.test(key);
}

/** One mention as TipTap writes it. `label` defaults to `id`. */
export function mentionSource(id: string, label = id): string {
  return `[@ id="${id}" label="${label}"]`;
}

/** Visible text for a workspace path: the last segment. */
export function fileLabel(path: string): string {
  const slash = path.replaceAll("\\", "/");
  const name = slash.slice(slash.lastIndexOf("/") + 1);
  return name || path;
}

/** One file mention as TipTap writes it. `label` defaults to the file name. */
export function fileMentionSource(path: string, label = fileLabel(path)): string {
  return `[@ file="${path}" label="${label}"]`;
}

/**
 * A workspace-relative path. `..`, `.`, an empty segment, and an absolute path
 * (leading slash or a drive letter) are not paths this library can check.
 */
export function isWorkspaceFileRef(path: string): boolean {
  const slash = path.replaceAll("\\", "/");
  if (!slash || slash.length > 512) return false;
  if (slash.startsWith("/")) return false;
  if (slash.charAt(1) === ":") return false;
  const parts = slash.split("/");
  return parts.every((part) => part.length > 0 && part !== "." && part !== "..");
}

function shortcodePattern(): RegExp {
  return new RegExp(SHORTCODE_SOURCE, "g");
}

function filePattern(): RegExp {
  return new RegExp(FILE_SHORTCODE_SOURCE, "g");
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
    const label = match[2] ?? "";
    out.push({ type: "ref", id, label: label || id });
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
  | { type: "file"; path: string; label: string };

/** Split one text run into prose, node mentions, and file mentions, in order. */
export function splitBodyRefs(text: string): BodySegment[] {
  const hits: Array<{
    start: number;
    end: number;
    kind: "ref" | "file";
    a: string;
    b: string;
  }> = [];
  for (const match of text.matchAll(shortcodePattern())) {
    const id = normalizeKey(match[1] ?? "");
    if (!id || !isKnowledgeKey(id)) continue;
    const start = match.index ?? 0;
    const label = match[2] ?? "";
    hits.push({
      start,
      end: start + match[0].length,
      kind: "ref",
      a: id,
      b: label || id,
    });
  }
  for (const match of text.matchAll(filePattern())) {
    const path = (match[1] ?? "").trim();
    if (!path) continue;
    const start = match.index ?? 0;
    const label = match[2] ?? "";
    hits.push({
      start,
      end: start + match[0].length,
      kind: "file",
      a: path,
      b: label || fileLabel(path),
    });
  }
  hits.sort((left, right) => left.start - right.start || left.end - right.end);
  const out: BodySegment[] = [];
  let last = 0;
  for (const hit of hits) {
    if (hit.start < last) continue;
    if (hit.start > last) out.push({ type: "text", value: text.slice(last, hit.start) });
    if (hit.kind === "ref") out.push({ type: "ref", id: hit.a, label: hit.b });
    else out.push({ type: "file", path: hit.a, label: hit.b });
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
    for (const match of line.matchAll(filePattern())) {
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
  return text.replace(shortcodePattern(), (full, id: string, label: string) => {
    const idKey = normalizeKey(id);
    const labelKey = normalizeKey(label);
    if (idKey !== source && labelKey !== source) return full;
    return mentionSource(idKey === source ? target : id, labelKey === source ? target : label);
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
  const nodes = text.replace(shortcodePattern(), (full, id: string, label: string) => {
    const key = normalizeKey(id);
    if (!isKnowledgeKey(key)) return full;
    return label || key;
  });
  return nodes.replace(filePattern(), (full, path: string, label: string) => {
    const cleaned = path.trim();
    if (!cleaned) return full;
    return label || fileLabel(cleaned);
  });
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

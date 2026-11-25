/**
 * Standard CommonMark Markdown link citations:
 * - File / directory / knowledge node: `[validate.rs](src/a.rs)` or `[seq](knowledge/contracts/seq.md)`
 * - Line / range: `[a.rs:4-9](src/a.rs#L4-L9)` (also accepts `#L4-9` and `#L4`)
 * - Symbol: `[Store.save](src/a.rs#Store.save)` (dot-separated identifier, `::` accepted)
 *
 * Rules match `src/knowledge/mentions.rs` and `tests/fixtures/knowledge_refs/cases.json` v3.
 */

const KNOWLEDGE_KEY = /^[\p{L}\p{N}_][\p{L}\p{N}_-]*(?: [\p{L}\p{N}_-]+)*$/u;
const MAX_LINE = 4294967295;

export function normalizeKey(key: string): string {
  return key.trim();
}

export function isKnowledgeKey(key: string): boolean {
  return KNOWLEDGE_KEY.test(key);
}

/** Filename stem without `.md` (case-insensitive), matching `document::file_stem` in Rust. */
export function fileStem(path: string): string {
  const norm = path.replaceAll("\\", "/");
  const base = norm.slice(norm.lastIndexOf("/") + 1);
  return base.toLowerCase().endsWith(".md") ? base.slice(0, -3) : base;
}

/** True when a node's declaration key is valid and matches its filename stem. */
export function isValidNodeIdentity(path: string, key: string): boolean {
  const normalized = normalizeKey(key);
  return Boolean(path) && isKnowledgeKey(normalized) && fileStem(path) === normalized;
}

/** Canonical logical knowledge path (`knowledge/<rel>`). */
export function logicalKnowledgePath(relPath: string): string {
  const clean = relPath.replaceAll("\\", "/").replace(/^\/+/, "").replace(/\/+$/, "");
  return clean ? `knowledge/${clean}` : "knowledge";
}

/** KR-PATH-2 path form used for identity and lookups. */
export function normalizeRefPath(path: string): string {
  return path.trim().replaceAll("\\", "/");
}

/** Normalize a symbol anchor query: trim and treat `::` as `.`. */
export function normalizeSymbolQuery(raw: string): string {
  return raw.trim().replaceAll("::", ".");
}

/**
 * A workspace-relative path. `..`, `.`, an empty segment, and an absolute path
 * (leading slash or drive letter) are not legal workspace citation paths.
 */
export function isWorkspaceFileRef(path: string): boolean {
  const slash = path.replaceAll("\\", "/");
  if (!slash || Array.from(slash).length > 512) return false;
  if (slash.startsWith("/")) return false;
  if (slash.charAt(1) === ":") return false;
  return slash.split("/").every((part) => part.length > 0 && part !== "." && part !== "..");
}

export function parseLineSpan(raw: string): { start: number; end: number } | null {
  const match = /^([1-9][0-9]*)(?:-([1-9][0-9]*))?$/.exec(raw);
  if (!match) return null;
  const start = Number(match[1]);
  const end = match[2] ? Number(match[2]) : start;
  if (start > MAX_LINE || end > MAX_LINE || end < start) return null;
  return { start, end };
}

/**
 * Parse a `#L...` line anchor (`L12`, `L12-30`, `L12-L30`).
 * Bare digits (`12`, `12-30`) without `L` are rejected.
 */
export function parseLineAnchor(raw: string): { start: number; end: number } | null {
  const match = /^L([1-9][0-9]*)(?:-L?([1-9][0-9]*))?$/.exec(raw);
  if (!match) return null;
  const start = Number(match[1]);
  const end = match[2] ? Number(match[2]) : start;
  if (start > MAX_LINE || end > MAX_LINE || end < start) return null;
  return { start, end };
}

/**
 * True when a `#fragment` looks like an attempted line range that failed
 * `parseLineAnchor` (so it reports `invalid_lines` instead of `missing_symbol`).
 */
export function isLineLikeAnchor(raw: string): boolean {
  if (!raw) return false;
  if (raw.startsWith("L")) {
    const rest = raw.slice(1);
    if (!rest) return false;
    const first = rest[0]!;
    return /[\p{Nd}\-+\s]/u.test(first);
  }
  let hasDigit = false;
  for (const ch of raw) {
    if (/\p{Nd}/u.test(ch)) {
      hasDigit = true;
    } else if (ch === "-" || ch === "+" || ch === " " || ch === "\t") {
      continue;
    } else {
      return false;
    }
  }
  return hasDigit;
}

export function formatLineSpan(start: number, end: number): string {
  return start === end ? String(start) : `${start}-${end}`;
}

export function formatLineAnchor(start: number, end: number): string {
  return start === end ? `L${start}` : `L${start}-L${end}`;
}

function pathParts(path: string): string[] {
  return path.replaceAll("\\", "/").split("/").filter((part) => part.length > 0);
}

/** Visible text for a workspace path: the last segment. */
export function fileLabel(path: string): string {
  const parts = pathParts(path);
  return parts[parts.length - 1] || path;
}

/** Final segment of a dot-separated symbol anchor (`Store.save` → `save`). */
export function leafSymbol(anchor: string): string {
  const norm = normalizeSymbolQuery(anchor);
  const parts = norm.split(".").filter((p) => p.length > 0);
  return parts[parts.length - 1] ?? norm;
}

export function humanFileLabel(path: string): string {
  return fileLabel(path);
}

export function humanSymbolLabel(path: string, symbol: string): string {
  return `${fileLabel(path)} : ${leafSymbol(symbol) || symbol.trim()}`;
}

export function humanRangeLabel(path: string, lines: string): string {
  const raw = lines.trim();
  const span = raw ? (parseLineSpan(raw) ?? parseLineAnchor(raw)) : null;
  if (!span) return fileLabel(path);
  return `${fileLabel(path)} : ${formatLineSpan(span.start, span.end)}`;
}

export function citationFact(
  path: string,
  symbol?: string | null,
  lines?: string | null,
): string {
  let out = path;
  const sym = symbol ? normalizeSymbolQuery(symbol) : "";
  if (sym) return `${out} : ${sym}`;
  const raw = lines?.trim() ?? "";
  const span = raw ? (parseLineSpan(raw) ?? parseLineAnchor(raw)) : null;
  if (span) out += ` : ${formatLineSpan(span.start, span.end)}`;
  return out;
}

export function formatLinkDest(dest: string): string {
  if (/[\s()<>]/.test(dest)) return `<${dest}>`;
  return dest;
}

/** Format one citation as a standard CommonMark inline link `[label](dest)`. */
export function citationMarkdown(input: {
  label?: string | null;
  path: string;
  symbol?: string | null;
  lines?: string | null;
  badLines?: string | null;
}): string {
  const path = normalizeRefPath(input.path);
  const symbol = input.symbol ? normalizeSymbolQuery(input.symbol) : "";
  const span =
    !symbol && input.lines ? (parseLineSpan(input.lines.trim()) ?? parseLineAnchor(input.lines.trim())) : null;
  const bad = !symbol && !span ? (input.badLines?.trim() ?? "") : "";

  let dest = path;
  if (symbol) dest += `#${symbol}`;
  else if (span) dest += `#${formatLineAnchor(span.start, span.end)}`;
  else if (bad) dest += `#${bad}`;

  const explicitLabel = input.label?.trim();
  const label =
    explicitLabel ||
    (symbol
      ? symbol
      : span
        ? `${fileLabel(path)}:${formatLineSpan(span.start, span.end)}`
        : fileLabel(path));

  return `[${label}](${formatLinkDest(dest)})`;
}

/**
 * `ref_key` (KR-KEY-1), the syntax-derived identity shared by frontend, backend,
 * and `cases.json` v3:
 * - `path:<norm_path>`
 * - `sym:<norm_path>\0<symbol>`
 * - `range:<norm_path>\0<lines_or_bad_lines>`
 */
export function knowledgeRefKey(ref: {
  path: string;
  symbol?: string | null;
  lines?: string | null;
  badLines?: string | null;
}): string {
  const path = normalizeRefPath(ref.path);
  const symbol = ref.symbol ? normalizeSymbolQuery(ref.symbol) : "";
  if (symbol) return `sym:${path}\0${symbol}`;
  const rawLines = (ref.lines ?? "").trim();
  if (rawLines) {
    const span = parseLineSpan(rawLines) ?? parseLineAnchor(rawLines);
    const normalized = span ? formatLineSpan(span.start, span.end) : rawLines;
    return `range:${path}\0${normalized}`;
  }
  const bad = (ref.badLines ?? "").trim();
  if (bad) return `range:${path}\0${bad}`;
  return `path:${path}`;
}

// ------------------------------------------------------------ link destination parsing

export interface ParsedWorkspaceDest {
  kind: "file" | "symbol" | "range";
  path: string;
  symbol: string | null;
  /** Normalized range span (`12` or `12-30`), or null. */
  lines: string | null;
  /** Raw malformed line fragment, or null. */
  badLines: string | null;
}

function schemeColonIndex(s: string): number {
  if (!s || !/^[A-Za-z]/.test(s)) return -1;
  for (let i = 1; i < s.length; i += 1) {
    const ch = s[i]!;
    if (ch === ":") return i;
    if (!/[A-Za-z0-9+.-]/.test(ch)) return -1;
  }
  return -1;
}

function safeDecodeURI(raw: string): string {
  if (!raw.includes("%")) return raw;
  try {
    return decodeURIComponent(raw);
  } catch {
    return raw;
  }
}

/**
 * Parse a Markdown link destination into a workspace citation target.
 * Returns `null` for web URLs (`https://`, `mailto:`, `//`), in-page `#anchors`,
 * and empty destinations.
 */
export function parseWorkspaceDest(destRaw: string): ParsedWorkspaceDest | null {
  let dest = destRaw.trim();
  if (!dest || dest.startsWith("//") || dest.startsWith("#")) return null;
  const colon = schemeColonIndex(dest);
  if (colon >= 0) {
    const scheme = dest.slice(0, colon);
    if (scheme.toLowerCase() === "file") {
      dest = dest.slice(colon + 1);
    } else if (scheme.length >= 2) {
      return null;
    }
    // Single-letter scheme (`C:/...`) stays so workspace path validation flags it.
  }

  const hashAt = dest.indexOf("#");
  const beforeHash = hashAt >= 0 ? dest.slice(0, hashAt) : dest;
  const fragRaw = hashAt >= 0 ? dest.slice(hashAt + 1) : null;
  const queryAt = beforeHash.indexOf("?");
  const pathRaw = queryAt >= 0 ? beforeHash.slice(0, queryAt) : beforeHash;

  const path = safeDecodeURI(pathRaw).trim();
  if (!path) return null;

  const frag = fragRaw != null ? safeDecodeURI(fragRaw).trim() : "";
  if (!frag) {
    return { kind: "file", path, symbol: null, lines: null, badLines: null };
  }

  const span = parseLineAnchor(frag);
  if (span) {
    return {
      kind: "range",
      path,
      symbol: null,
      lines: formatLineSpan(span.start, span.end),
      badLines: null,
    };
  }
  if (isLineLikeAnchor(frag)) {
    return { kind: "range", path, symbol: null, lines: null, badLines: frag };
  }
  return {
    kind: "symbol",
    path,
    symbol: normalizeSymbolQuery(frag),
    lines: null,
    badLines: null,
  };
}

// ------------------------------------------------------------ CommonMark inline link scanner

function maskInlineCodeSameLen(line: string): string {
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

function isEscaped(text: string, pos: number): boolean {
  let backslashes = 0;
  let k = pos;
  while (k > 0 && text[k - 1] === "\\") {
    backslashes += 1;
    k -= 1;
  }
  return backslashes % 2 === 1;
}

function findClosingBracket(text: string, from: number): number {
  let depth = 1;
  let i = from;
  while (i < text.length) {
    const ch = text[i]!;
    if (ch === "\\" && i + 1 < text.length) {
      i += 2;
      continue;
    }
    if (ch === "[") depth += 1;
    else if (ch === "]") {
      depth -= 1;
      if (depth === 0) return i;
    }
    i += 1;
  }
  return -1;
}

function parseLinkDestination(
  text: string,
  from: number,
): { destStart: number; destEnd: number; closeParen: number } | null {
  let i = from;
  while (i < text.length && (text[i] === " " || text[i] === "\t")) i += 1;
  if (i >= text.length || text[i] === ")") return null;

  let destStart: number;
  let destEnd: number;
  let afterDest: number;

  if (text[i] === "<") {
    destStart = i + 1;
    let j = destStart;
    while (j < text.length) {
      if (text[j] === "\\" && j + 1 < text.length) {
        j += 2;
        continue;
      }
      if (text[j] === "<") return null;
      if (text[j] === ">") break;
      j += 1;
    }
    if (j >= text.length || text[j] !== ">") return null;
    destEnd = j;
    afterDest = j + 1;
  } else {
    destStart = i;
    let j = destStart;
    let parenDepth = 0;
    while (j < text.length) {
      const ch = text[j]!;
      if (ch === "\\" && j + 1 < text.length) {
        j += 2;
        continue;
      }
      if (ch === " " || ch === "\t") break;
      if (ch === "(") parenDepth += 1;
      else if (ch === ")") {
        if (parenDepth === 0) break;
        parenDepth -= 1;
      }
      j += 1;
    }
    destEnd = j;
    afterDest = j;
  }

  if (destEnd <= destStart) return null;
  while (afterDest < text.length && (text[afterDest] === " " || text[afterDest] === "\t")) {
    afterDest += 1;
  }
  if (afterDest < text.length && (text[afterDest] === '"' || text[afterDest] === "'")) {
    const quote = text[afterDest]!;
    afterDest += 1;
    while (afterDest < text.length) {
      if (text[afterDest] === "\\" && afterDest + 1 < text.length) {
        afterDest += 2;
        continue;
      }
      if (text[afterDest] === quote) {
        afterDest += 1;
        break;
      }
      afterDest += 1;
    }
    while (afterDest < text.length && (text[afterDest] === " " || text[afterDest] === "\t")) {
      afterDest += 1;
    }
  }
  if (afterDest < text.length && text[afterDest] === ")") {
    return { destStart, destEnd, closeParen: afterDest };
  }
  return null;
}

interface ScannedLink {
  start: number;
  end: number;
  label: string;
  raw: string;
  parsed: ParsedWorkspaceDest | null;
}

function scanLinksInLine(line: string, prose: string): ScannedLink[] {
  const hits: ScannedLink[] = [];
  let i = 0;
  while (i < prose.length) {
    if (prose[i] !== "[") {
      i += 1;
      continue;
    }
    if ((i > 0 && prose[i - 1] === "!") || isEscaped(prose, i)) {
      i += 1;
      continue;
    }
    const closeBracket = findClosingBracket(prose, i + 1);
    if (closeBracket < 0 || prose[closeBracket + 1] !== "(") {
      i += 1;
      continue;
    }
    const dest = parseLinkDestination(prose, closeBracket + 2);
    if (!dest) {
      i += 1;
      continue;
    }
    const label = line.slice(i + 1, closeBracket);
    const destRaw = line.slice(dest.destStart, dest.destEnd);
    const raw = line.slice(i, dest.closeParen + 1);
    hits.push({
      start: i,
      end: dest.closeParen + 1,
      label,
      raw,
      parsed: parseWorkspaceDest(destRaw),
    });
    i = dest.closeParen + 1;
  }
  return hits;
}

function scanProseWithSource(markdown: string): Array<{ line: string; prose: string }> {
  const out: Array<{ line: string; prose: string }> = [];
  let fenceChar = "";
  let fenceLen = 0;
  for (const raw of markdown.split("\n")) {
    const line = raw.endsWith("\r") ? raw.slice(0, -1) : raw;
    if (fenceLen > 0) {
      const closed = new RegExp(`^ {0,3}${fenceChar}{${fenceLen},}\\s*$`).test(line);
      if (closed) fenceLen = 0;
      continue;
    }
    const open = /^ {0,3}(`{3,}|~{3,})/.exec(line);
    if (open) {
      fenceChar = open[1]![0]!;
      fenceLen = open[1]!.length;
      continue;
    }
    out.push({ line, prose: maskInlineCodeSameLen(line) });
  }
  return out;
}

export type BodySegment =
  | { type: "text"; value: string }
  | {
      type: "cite";
      label: string;
      path: string;
      symbol: string | null;
      lines: string | null;
      badLines: string | null;
      raw: string;
    };

/** Split one line of prose into literal pieces and workspace Markdown links. */
export function splitBodyRefs(text: string): BodySegment[] {
  const prose = maskInlineCodeSameLen(text);
  const hits = scanLinksInLine(text, prose);
  const out: BodySegment[] = [];
  let last = 0;
  for (const hit of hits) {
    if (!hit.parsed) continue;
    if (hit.start > last) out.push({ type: "text", value: text.slice(last, hit.start) });
    out.push({
      type: "cite",
      label: hit.label,
      path: hit.parsed.path,
      symbol: hit.parsed.symbol,
      lines: hit.parsed.lines,
      badLines: hit.parsed.badLines,
      raw: hit.raw,
    });
    last = hit.end;
  }
  if (last < text.length) out.push({ type: "text", value: text.slice(last) });
  if (out.length === 0) out.push({ type: "text", value: text });
  return out;
}

export interface ExtractedRef {
  kind: "file" | "symbol" | "range";
  path: string;
  symbol?: string;
  lines?: string | null;
  bad_lines?: string;
  ref_key: string;
}

/**
 * Every workspace citation in prose, in document order, repeats kept (KR-SYN-8).
 * Shapes match `tests/fixtures/knowledge_refs/cases.json` v3 `extract[].refs`.
 */
export function extractRefs(markdown: string): ExtractedRef[] {
  const out: ExtractedRef[] = [];
  for (const { line, prose } of scanProseWithSource(markdown)) {
    for (const hit of scanLinksInLine(line, prose)) {
      const p = hit.parsed;
      if (!p) continue;
      const path = normalizeRefPath(p.path);
      const ref_key = knowledgeRefKey({
        path,
        symbol: p.symbol,
        lines: p.lines,
        badLines: p.badLines,
      });
      if (p.kind === "symbol" && p.symbol) {
        out.push({ kind: "symbol", path, symbol: p.symbol, ref_key });
      } else if (p.kind === "range") {
        out.push(
          p.badLines != null
            ? { kind: "range", path, lines: null, bad_lines: p.badLines, ref_key }
            : { kind: "range", path, lines: p.lines, ref_key },
        );
      } else {
        out.push({ kind: "file", path, ref_key });
      }
    }
  }
  return out;
}

/**
 * Fold bare-path citations in `markdown` to valid target node keys (first-seen order, deduped).
 * Excludes self-relations when `ownPath` is provided.
 */
export function extractNodeRelations(
  markdown: string,
  lookupNode: (normPath: string) => { key: string; path: string } | undefined,
  ownPath?: string,
): string[] {
  const keys: string[] = [];
  const seen = new Set<string>();
  for (const ref of extractRefs(markdown)) {
    if (ref.kind !== "file") continue;
    const target = lookupNode(ref.path);
    if (!target) continue;
    if (ownPath && target.path === ownPath) continue;
    const key = normalizeKey(target.key);
    if (!key || seen.has(key)) continue;
    seen.add(key);
    keys.push(key);
  }
  return keys;
}

/** Rewrite workspace links targeting `fromPath` to `toPath`, skipping fenced & inline code. */
export function replaceCitationPath(
  markdown: string,
  fromPath: string,
  toPath: string,
): string {
  const fromNorm = normalizeRefPath(fromPath);
  const toNorm = normalizeRefPath(toPath);
  if (!fromNorm || fromNorm === toNorm) return markdown;
  const oldStem = fileStem(fromNorm);
  const newStem = fileStem(toNorm);
  const oldBase = fileLabel(fromNorm);
  const newBase = fileLabel(toNorm);
  return rewriteProseLinks(markdown, (hit) => {
    const p = hit.parsed;
    if (!p || normalizeRefPath(p.path) !== fromNorm) return null;
    const nextLabel =
      hit.label === oldStem ? newStem : hit.label === oldBase ? newBase : hit.label;
    return citationMarkdown({
      label: nextLabel,
      path: toNorm,
      symbol: p.symbol,
      lines: p.lines,
      badLines: p.badLines,
    });
  });
}

/** Rewrite workspace links under `fromPrefix/` to `toPrefix/`, skipping fenced & inline code. */
export function replaceCitationPathPrefix(
  markdown: string,
  fromPrefix: string,
  toPrefix: string,
): string {
  const fromClean = normalizeRefPath(fromPrefix).replace(/\/+$/, "");
  const toClean = normalizeRefPath(toPrefix).replace(/\/+$/, "");
  if (!fromClean || fromClean === toClean) return markdown;
  const prefixSlash = `${fromClean}/`;
  return rewriteProseLinks(markdown, (hit) => {
    const p = hit.parsed;
    if (!p) return null;
    const norm = normalizeRefPath(p.path);
    if (!norm.startsWith(prefixSlash)) return null;
    const rest = norm.slice(prefixSlash.length);
    const nextPath = toClean ? `${toClean}/${rest}` : rest;
    return citationMarkdown({
      label: hit.label,
      path: nextPath,
      symbol: p.symbol,
      lines: p.lines,
      badLines: p.badLines,
    });
  });
}

function rewriteProseLinks(
  markdown: string,
  rewrite: (hit: ScannedLink) => string | null,
): string {
  const lines = markdown.split("\n");
  let fenceChar = "";
  let fenceLen = 0;
  const out: string[] = [];
  for (const raw of lines) {
    const hasCr = raw.endsWith("\r");
    const line = hasCr ? raw.slice(0, -1) : raw;
    if (fenceLen > 0) {
      const closed = new RegExp(`^ {0,3}${fenceChar}{${fenceLen},}\\s*$`).test(line);
      if (closed) fenceLen = 0;
      out.push(raw);
      continue;
    }
    const open = /^ {0,3}(`{3,}|~{3,})/.exec(line);
    if (open) {
      fenceChar = open[1]![0]!;
      fenceLen = open[1]!.length;
      out.push(raw);
      continue;
    }
    const prose = maskInlineCodeSameLen(line);
    const hits = scanLinksInLine(line, prose);
    let rebuilt = "";
    let last = 0;
    for (const hit of hits) {
      const replacement = rewrite(hit);
      if (replacement == null) continue;
      rebuilt += line.slice(last, hit.start) + replacement;
      last = hit.end;
    }
    rebuilt += line.slice(last);
    out.push(hasCr ? `${rebuilt}\r` : rebuilt);
  }
  return out.join("\n");
}

// ------------------------------------------------------------ preview

const PREVIEW_CODE_CHARS = 24;

function clipChars(value: string, max: number): string {
  const chars = Array.from(value);
  if (chars.length <= max) return value;
  return `${chars.slice(0, max).join("")}…`;
}

function replaceLinksWithLabels(text: string): string {
  const prose = maskInlineCodeSameLen(text);
  const hits = scanLinksInLine(text, prose);
  if (hits.length === 0) return text;
  let out = "";
  let last = 0;
  for (const hit of hits) {
    if (!hit.parsed) continue;
    out += text.slice(last, hit.start);
    const label = hit.label.trim();
    out +=
      label ||
      citationFact(
        normalizeRefPath(hit.parsed.path),
        hit.parsed.symbol,
        hit.parsed.lines,
      );
    last = hit.end;
  }
  out += text.slice(last);
  return out;
}

function showInlineCode(line: string): string {
  let out = "";
  let text = "";
  let i = 0;
  const flush = () => {
    if (!text) return;
    out += replaceLinksWithLabels(text);
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

/** First lines of a body, with workspace links reduced to their labels, for card previews. */
export function knowledgePreview(value: string, lines = 3): string {
  const prose: string[] = [];
  let fenceChar = "";
  let fenceLen = 0;
  for (const raw of value.split("\n")) {
    const line = raw.endsWith("\r") ? raw.slice(0, -1) : raw;
    if (fenceLen > 0) {
      const closed = new RegExp(`^ {0,3}${fenceChar}{${fenceLen},}\\s*$`).test(line);
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
  return prose
    .map((line) => line.trim())
    .filter((line) => line.length > 0)
    .slice(0, lines)
    .join("\n");
}

export type KnowledgeSegment =
  | { type: "text"; value: string }
  | { type: "ref"; key: string };

/** Compare and look up keys after trimming. The stored key is left unchanged. */
export function normalizeKey(key: string): string {
  return key.trim();
}

function refPattern(): RegExp {
  return /\[\[([^\]\n]+?)\]\]/g;
}

/**
 * Split one text run into literal pieces and `[[key]]` markers.
 * Whitespace-only markers stay literal text.
 */
export function splitKnowledgeRefs(text: string): KnowledgeSegment[] {
  const out: KnowledgeSegment[] = [];
  let last = 0;
  for (const match of text.matchAll(refPattern())) {
    const key = normalizeKey(match[1] ?? "");
    const start = match.index ?? 0;
    if (!key) continue;
    if (start > last) out.push({ type: "text", value: text.slice(last, start) });
    out.push({ type: "ref", key });
    last = start + match[0].length;
  }
  if (last < text.length) out.push({ type: "text", value: text.slice(last) });
  if (out.length === 0) out.push({ type: "text", value: text });
  return out;
}

/** Replace an inline code span with spaces so markers inside it disappear. */
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

/**
 * Keys cited in prose. Fenced blocks and inline code are not citations —
 * the same exclusion the markdown renderer gets from the AST.
 */
export function extractMarkers(markdown: string): string[] {
  const keys: string[] = [];
  for (const line of scanProse(markdown).prose) {
    for (const segment of splitKnowledgeRefs(line)) {
      if (segment.type === "ref") keys.push(segment.key);
    }
  }
  return keys;
}

/** First non-empty prose line as literal/ref segments (for one-line summaries). */
export function knowledgeFirstLineSegments(value: string): KnowledgeSegment[] {
  for (const line of scanProse(value).prose) {
    if (!line.trim()) continue;
    return splitKnowledgeRefs(line);
  }
  return [{ type: "text", value: "" }];
}

/** First lines of a value, with markers reduced to their key, for card previews. */
export function knowledgePreview(value: string, lines = 3): string {
  return scanProse(value)
    .prose.join("\n")
    .replace(refPattern(), (_match, key: string) => normalizeKey(key))
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line.length > 0)
    .slice(0, lines)
    .join("\n");
}

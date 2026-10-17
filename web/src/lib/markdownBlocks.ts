/** Merge short paragraphs up to this size so a block is not one line. */
export const MARKDOWN_BLOCK_MIN_CHARS = 800;
/** Force a cut once a block passes this size. */
export const MARKDOWN_BLOCK_MAX_CHARS = 2500;

/** Same reading measure the transcript uses for unmeasured bubbles. */
const CHARS_PER_LINE = 72;
const LINE_HEIGHT_PX = 25;

export interface MarkdownBlock {
  text: string;
  estimatedHeight: number;
}

interface Fence {
  char: "`" | "~";
  len: number;
}

interface LineSpan {
  from: number;
  to: number;
  next: number;
}

/**
 * Split markdown into blocks the renderer can mount independently.
 *
 * A block closes at a blank line outside a fence once it reaches
 * {@link MARKDOWN_BLOCK_MIN_CHARS}. Past {@link MARKDOWN_BLOCK_MAX_CHARS} it
 * closes at the previous newline, or on a character boundary when the run has
 * no newline. Appended text only changes the tail: earlier blocks stay put.
 */
export function splitMarkdownBlocks(text: string): MarkdownBlock[] {
  if (text.length === 0) {
    return [{ text: "", estimatedHeight: estimateBlockHeight("") }];
  }

  const blocks: MarkdownBlock[] = [];
  let blockStart = 0;

  while (blockStart < text.length) {
    const cut = nextCut(text, blockStart);
    if (cut === null) {
      blocks.push(blockOf(text.slice(blockStart)));
      break;
    }
    blocks.push(blockOf(text.slice(blockStart, cut)));
    blockStart = cut;
  }

  return blocks;
}

/** Rough pixel height from wrapped line count, before the slot is measured. */
export function estimateBlockHeight(text: string): number {
  if (text.length === 0) return LINE_HEIGHT_PX;
  let lines = 0;
  let start = 0;
  for (let i = 0; i <= text.length; i += 1) {
    if (i !== text.length && text[i] !== "\n") continue;
    const len = i - start;
    lines += Math.max(1, Math.ceil(len / CHARS_PER_LINE));
    start = i + 1;
  }
  return lines * LINE_HEIGHT_PX;
}

function blockOf(text: string): MarkdownBlock {
  return { text, estimatedHeight: estimateBlockHeight(text) };
}

/** Exclusive end of the next closed block, or null when the rest stays open. */
function nextCut(text: string, blockStart: number): number | null {
  let fence = fenceAt(text, blockStart);
  let index = blockStart;

  while (index < text.length) {
    const line = readLine(text, index);
    const content = text.slice(line.from, line.to);
    const openBefore = fence;
    if (fence === null) fence = openingFence(content);
    else if (closesFence(content, fence)) fence = null;

    const end = line.next;
    const size = end - blockStart;
    const blankOutside =
      openBefore === null && fence === null && isBlankLine(content);

    if (
      blankOutside &&
      size >= MARKDOWN_BLOCK_MIN_CHARS &&
      size <= MARKDOWN_BLOCK_MAX_CHARS
    ) {
      return end;
    }

    if (size > MARKDOWN_BLOCK_MAX_CHARS) {
      const cut = forceCut(text, blockStart);
      return cut >= text.length ? null : cut;
    }

    if (line.next === index) return null;
    index = end;
  }

  return null;
}

function forceCut(text: string, start: number): number {
  const limit = start + MARKDOWN_BLOCK_MAX_CHARS;
  if (limit >= text.length) return text.length;
  for (let i = limit - 1; i > start; i -= 1) {
    if (text[i] === "\n") return i + 1;
  }
  return hardCut(text, start);
}

/** Cut on a code-point boundary so a surrogate pair stays in one block. */
function hardCut(text: string, start: number): number {
  let end = Math.min(text.length, start + MARKDOWN_BLOCK_MAX_CHARS);
  if (end >= text.length) return text.length;
  const prev = text.charCodeAt(end - 1);
  if (prev >= 0xd800 && prev <= 0xdbff) end += 1;
  return Math.min(end, text.length);
}

function fenceAt(text: string, index: number): Fence | null {
  let fence: Fence | null = null;
  let cursor = 0;
  while (cursor < index) {
    const line = readLine(text, cursor);
    if (line.from >= index) break;
    // A cut that lands mid-line has not consumed that line's fence marker.
    if (line.to > index) break;
    const content = text.slice(line.from, line.to);
    if (fence === null) fence = openingFence(content);
    else if (closesFence(content, fence)) fence = null;
    if (line.next === cursor) break;
    cursor = line.next;
  }
  return fence;
}

function readLine(text: string, index: number): LineSpan {
  const from = index;
  let to = index;
  while (to < text.length && text[to] !== "\n") to += 1;
  return { from, to, next: to < text.length ? to + 1 : to };
}

function isBlankLine(line: string): boolean {
  for (let i = 0; i < line.length; i += 1) {
    const code = line.charCodeAt(i);
    if (code !== 0x20 && code !== 0x09 && code !== 0x0d) return false;
  }
  return true;
}

/** Opening ``` / ~~~ fence, indented at most three spaces. */
function openingFence(line: string): Fence | null {
  return fenceMarker(line);
}

function closesFence(line: string, fence: Fence): boolean {
  const marker = fenceMarker(line);
  if (marker === null || marker.char !== fence.char || marker.len < fence.len) {
    return false;
  }
  let index = 0;
  while (index < line.length && index < 3 && line[index] === " ") index += 1;
  index += marker.len;
  while (index < line.length) {
    const code = line.charCodeAt(index);
    if (code !== 0x20 && code !== 0x09 && code !== 0x0d) return false;
    index += 1;
  }
  return true;
}

function fenceMarker(line: string): Fence | null {
  let index = 0;
  while (index < line.length && index < 3 && line[index] === " ") index += 1;
  if (index >= line.length) return null;
  const char = line[index];
  if (char !== "`" && char !== "~") return null;
  let len = 0;
  while (index < line.length && line[index] === char) {
    len += 1;
    index += 1;
  }
  if (len < 3) return null;
  return { char, len };
}

/**
 * Split markdown into blocks that can be memoized independently.
 *
 * A finished block is closed only on a real markdown boundary: a blank line
 * outside a fence, list, quote, table, or indented code block, or a block
 * that interrupts a paragraph (heading, fence, list, quote, thematic break).
 * An open fence, a loose list, and a block quote keep their internal blank
 * lines, so a later chunk cannot move an earlier cut. Link definitions and
 * footnotes stay one block, because a reference only resolves in the same
 * parse as its definition.
 *
 * Appending text changes the tail. Earlier blocks keep their text, so a
 * memoized renderer re-parses only the tail. Blocks are not cut on a
 * character count: a long paragraph or fence stays whole.
 */

const LINK_DEFINITION = /^ {0,3}\[[^\]\n]+\]:/m;
const FOOTNOTE_REFERENCE = /\[\^[\w-]{1,200}\](?!:)/;
const FOOTNOTE_DEFINITION = /\[\^[\w-]{1,200}\]:/;

interface Line {
  from: number;
  to: number;
  next: number;
  content: string;
}

interface Fence {
  char: "`" | "~";
  len: number;
}

export function splitMarkdownBlocks(text: string): string[] {
  if (text.length === 0) return [""];
  if (hasCrossBlockReference(text)) return [text];

  const blocks: string[] = [];
  let cursor = 0;
  while (cursor < text.length) {
    const end = findBlockEnd(text, cursor);
    if (end <= cursor) {
      blocks.push(text.slice(cursor));
      break;
    }
    blocks.push(text.slice(cursor, end));
    cursor = end;
  }
  return blocks.length > 0 ? blocks : [text];
}

/** Reference links and footnotes resolve only inside one parse. */
function hasCrossBlockReference(text: string): boolean {
  return (
    LINK_DEFINITION.test(text) ||
    FOOTNOTE_REFERENCE.test(text) ||
    FOOTNOTE_DEFINITION.test(text)
  );
}

/**
 * End of the block that starts at `start`, including the blank line that
 * separates it from the next block. Leading blank lines belong to this block.
 */
function findBlockEnd(text: string, start: number): number {
  let cursor = start;
  while (cursor < text.length) {
    const line = readLine(text, cursor);
    if (!isBlankLine(line.content)) break;
    cursor = line.next;
  }
  if (cursor >= text.length) return text.length;
  return endOfBlock(text, cursor);
}

function endOfBlock(text: string, start: number): number {
  const first = readLine(text, start);
  const fence = openingFence(first.content);
  if (fence) return afterFence(text, first.next, fence);
  if (isAtxHeading(first.content) || isThematicBreak(first.content)) {
    return separatorEnd(text, first.next);
  }
  if (isBlockquoteLine(first.content)) return endQuote(text, start);
  const column = listContentColumn(first.content);
  if (column !== null) return endList(text, start, column);
  if (isIndentedCode(first.content)) return endIndented(text, start);
  return endParagraph(text, start);
}

/** Include a following run of blank lines when it separates another block. */
function separatorEnd(text: string, index: number): number {
  let cursor = index;
  let blanks = false;
  while (cursor < text.length) {
    const line = readLine(text, cursor);
    if (!isBlankLine(line.content)) break;
    blanks = true;
    cursor = line.next;
  }
  return blanks ? cursor : index;
}

function nextContent(text: string, index: number): number | null {
  let cursor = index;
  while (cursor < text.length) {
    const line = readLine(text, cursor);
    if (!isBlankLine(line.content)) return line.from;
    cursor = line.next;
  }
  return null;
}

function afterFence(text: string, index: number, fence: Fence): number {
  let cursor = index;
  while (cursor < text.length) {
    const line = readLine(text, cursor);
    if (closesFence(line.content, fence)) return separatorEnd(text, line.next);
    if (line.next === cursor) return text.length;
    cursor = line.next;
  }
  return text.length;
}

function endQuote(text: string, start: number): number {
  let cursor = start;
  while (cursor < text.length) {
    const line = readLine(text, cursor);
    if (isBlankLine(line.content)) {
      const next = nextContent(text, line.next);
      if (next === null) return text.length;
      if (isBlockquoteLine(readLine(text, next).content)) {
        cursor = line.next;
        continue;
      }
      return separatorEnd(text, line.from);
    }
    cursor = line.next;
  }
  return text.length;
}

function endList(text: string, start: number, contentColumn: number): number {
  let cursor = start;
  let required = contentColumn;
  let firstLine = true;
  while (cursor < text.length) {
    const line = readLine(text, cursor);
    if (isBlankLine(line.content)) {
      const next = nextContent(text, line.next);
      if (next === null) return text.length;
      const nextLine = readLine(text, next).content;
      const sibling = listContentColumn(nextLine);
      if (sibling !== null) {
        required = Math.min(required, sibling);
        cursor = line.next;
        continue;
      }
      if (indentColumns(nextLine) >= required) {
        cursor = line.next;
        continue;
      }
      return separatorEnd(text, line.from);
    }
    if (!firstLine) {
      const sibling = listContentColumn(line.content);
      if (sibling !== null) required = Math.min(required, sibling);
      else if (indentColumns(line.content) < required) return cursor;
    }
    firstLine = false;
    cursor = line.next;
  }
  return text.length;
}

function endIndented(text: string, start: number): number {
  let cursor = start;
  while (cursor < text.length) {
    const line = readLine(text, cursor);
    if (isBlankLine(line.content)) {
      const next = nextContent(text, line.next);
      if (next === null) return text.length;
      if (isIndentedCode(readLine(text, next).content)) {
        cursor = line.next;
        continue;
      }
      return separatorEnd(text, line.from);
    }
    if (cursor !== start && !isIndentedCode(line.content)) return cursor;
    cursor = line.next;
  }
  return text.length;
}

function endParagraph(text: string, start: number): number {
  let cursor = start;
  let lines = 0;
  let previous: Line | null = null;
  while (cursor < text.length) {
    const line = readLine(text, cursor);
    if (lines > 0 && isBlankLine(line.content)) {
      return separatorEnd(text, line.from);
    }
    if (
      lines > 0 &&
      previous &&
      isTableDelimiter(line.content) &&
      looksLikeTableRow(previous.content)
    ) {
      if (lines === 1) return endTable(text, line.next);
      return previous.from;
    }
    if (lines > 0 && previous && isSetextUnderline(line.content)) {
      return separatorEnd(text, line.next);
    }
    if (lines > 0 && startsNewBlock(line.content)) return cursor;
    previous = line;
    lines += 1;
    cursor = line.next;
  }
  return text.length;
}

function endTable(text: string, index: number): number {
  let cursor = index;
  while (cursor < text.length) {
    const line = readLine(text, cursor);
    if (isBlankLine(line.content)) return separatorEnd(text, line.from);
    if (!looksLikeTableRow(line.content)) return cursor;
    cursor = line.next;
  }
  return text.length;
}

function startsNewBlock(line: string): boolean {
  return (
    openingFence(line) !== null ||
    isAtxHeading(line) ||
    isThematicBreak(line) ||
    isBlockquoteLine(line) ||
    listContentColumn(line) !== null
  );
}

function readLine(text: string, index: number): Line {
  const from = index;
  let to = index;
  while (to < text.length && text[to] !== "\n" && text[to] !== "\r") to += 1;
  let next = to;
  if (to < text.length) {
    next = to + 1;
    if (text[to] === "\r" && text[next] === "\n") next += 1;
  }
  return { from, to, next, content: text.slice(from, to) };
}

function isBlankLine(line: string): boolean {
  for (let i = 0; i < line.length; i += 1) {
    const code = line.charCodeAt(i);
    if (code !== 0x20 && code !== 0x09) return false;
  }
  return true;
}

function indentColumns(line: string): number {
  let cols = 0;
  for (let i = 0; i < line.length; i += 1) {
    const ch = line[i];
    if (ch === " ") cols += 1;
    else if (ch === "\t") cols += 4;
    else break;
  }
  return cols;
}

function isAtxHeading(line: string): boolean {
  let index = 0;
  while (index < line.length && index < 3 && line[index] === " ") index += 1;
  let hashes = 0;
  while (index < line.length && line[index] === "#" && hashes < 7) {
    hashes += 1;
    index += 1;
  }
  if (hashes < 1 || hashes > 6) return false;
  return index >= line.length || line[index] === " " || line[index] === "\t";
}

function isThematicBreak(line: string): boolean {
  let index = 0;
  while (index < line.length && index < 3 && line[index] === " ") index += 1;
  if (index >= line.length) return false;
  const char = line[index];
  if (char !== "-" && char !== "_" && char !== "*") return false;
  let count = 0;
  for (; index < line.length; index += 1) {
    const current = line[index];
    if (current === char) count += 1;
    else if (current !== " " && current !== "\t") return false;
  }
  return count >= 3;
}

function isSetextUnderline(line: string): boolean {
  let index = 0;
  while (index < line.length && index < 3 && line[index] === " ") index += 1;
  if (index >= line.length) return false;
  const char = line[index];
  if (char !== "=" && char !== "-") return false;
  for (; index < line.length; index += 1) {
    const current = line[index];
    if (current === char || current === " " || current === "\t") continue;
    return false;
  }
  return true;
}

function isBlockquoteLine(line: string): boolean {
  let index = 0;
  while (index < line.length && index < 3 && line[index] === " ") index += 1;
  return line[index] === ">";
}

function isIndentedCode(line: string): boolean {
  return line.startsWith("\t") || (line.startsWith("    ") && !isBlankLine(line));
}

function looksLikeTableRow(line: string): boolean {
  return line.includes("|");
}

function isTableDelimiter(line: string): boolean {
  if (!line.includes("|")) return false;
  const trimmed = line.trim().replace(/^\|/, "").replace(/\|$/, "");
  const cells = trimmed.split("|");
  return (
    cells.length > 0 && cells.every((cell) => /^:?-{3,}:?$/.test(cell.trim()))
  );
}

/** Column where a list item's content starts, or null when the line is not an item. */
function listContentColumn(line: string): number | null {
  if (isThematicBreak(line)) return null;
  let index = 0;
  let column = 0;
  while (index < line.length && column < 3 && line[index] === " ") {
    index += 1;
    column += 1;
  }
  if (index >= line.length) return null;

  const bullet = line[index];
  if (bullet === "-" || bullet === "+" || bullet === "*") {
    return markerContentColumn(line, index + 1, column + 1);
  }

  let digits = 0;
  let cursor = index;
  while (cursor < line.length && digits < 9 && isDigit(line[cursor])) {
    cursor += 1;
    digits += 1;
  }
  if (digits === 0 || (line[cursor] !== "." && line[cursor] !== ")")) return null;
  return markerContentColumn(line, cursor + 1, column + digits + 1);
}

function markerContentColumn(
  line: string,
  index: number,
  column: number,
): number | null {
  if (index >= line.length) return column + 1;
  if (line[index] !== " " && line[index] !== "\t") return null;
  let cursor = index;
  let cols = column;
  let spaces = 0;
  while (
    cursor < line.length &&
    spaces < 4 &&
    (line[cursor] === " " || line[cursor] === "\t")
  ) {
    cols += line[cursor] === " " ? 1 : 4;
    cursor += 1;
    spaces += 1;
  }
  return cols;
}

function isDigit(char: string | undefined): boolean {
  return char !== undefined && char >= "0" && char <= "9";
}

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
    if (code !== 0x20 && code !== 0x09) return false;
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

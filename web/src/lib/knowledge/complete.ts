export interface KnowledgeCompletion {
  /** Index in `text` where the completed token starts. */
  start: number;
  /** Index where the completed token currently ends (the caret). */
  end: number;
  items: string[];
}

function filterKeys(candidates: string[], query: string): string[] {
  return candidates.filter((key) => key.startsWith(query) && key !== query);
}

/**
 * Completion for a `ref :` line or a `[[node : key]]` marker at the caret.
 * Candidates for the body are the file's own declared refs.
 */
export function completionAt(
  kind: "refs" | "body",
  text: string,
  caret: number,
  candidates: string[],
): KnowledgeCompletion | null {
  const safeCaret = Math.max(0, Math.min(caret, text.length));
  if (kind === "refs") {
    const lineStart = text.lastIndexOf("\n", safeCaret - 1) + 1;
    const line = text.slice(lineStart, safeCaret);
    const match = /^(ref\s*:\s*)([^/\n]*)$/.exec(line);
    if (!match) return null;
    const query = match[2] ?? "";
    const items = filterKeys(candidates, query);
    if (items.length === 0) return null;
    return { start: lineStart + (match[1]?.length ?? 0), end: safeCaret, items };
  }
  const before = text.slice(0, safeCaret);
  const match = /\[\[\s*node\s*:?\s*([^\]\n]*)$/.exec(before);
  if (!match) return null;
  const query = match[1] ?? "";
  const items = filterKeys(candidates, query);
  if (items.length === 0) return null;
  const token = match[0] ?? "";
  return { start: safeCaret - token.length, end: safeCaret, items };
}

/** Insert a chosen key, closing a body marker when the caret is inside one. */
export function applyCompletion(
  kind: "refs" | "body",
  text: string,
  completion: KnowledgeCompletion,
  key: string,
): { text: string; caret: number } {
  const inserted = kind === "body" ? `[[node : ${key}]]` : key;
  const next = text.slice(0, completion.start) + inserted + text.slice(completion.end);
  return { text: next, caret: completion.start + inserted.length };
}

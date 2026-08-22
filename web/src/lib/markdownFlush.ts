/** Live Milkdown instances register a flush so Ctrl+S can read the latest doc. */
const flushers = new Map<string, () => string | null>();

export function registerMarkdownFlush(
  path: string,
  flush: () => string | null,
): () => void {
  flushers.set(path, flush);
  return () => {
    if (flushers.get(path) === flush) flushers.delete(path);
  };
}

export function flushMarkdownEditor(path: string): string | null {
  return flushers.get(path)?.() ?? null;
}

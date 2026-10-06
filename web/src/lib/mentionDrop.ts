import { fetchSymbolAt } from "../api/workspace";
import {
  chipPath,
  LITECODE_SPAN_MIME,
  osFilePath,
  readCodeSpan,
  readLoosePaths,
  readTreePaths,
} from "./dropPayload";
import { fileMentionSource, formatLineSpan, symbolMentionSource } from "./knowledge/markers";

function chips(paths: string[], projectRoot: string): string | null {
  const sources = paths
    .map((path) => chipPath(path, projectRoot))
    .filter((path) => path.length > 0)
    .map((path) => fileMentionSource(path));
  return sources.length > 0 ? sources.join(" ") : null;
}

/**
 * Shortcodes to insert for a conversation drop.
 * File-tree paths, a Monaco span, OS files, and loose path text — in that order.
 */
export async function mentionTextForDrop(
  dt: DataTransfer,
  projectRoot: string,
): Promise<string | null> {
  if (Array.from(dt.types).includes(LITECODE_SPAN_MIME)) {
    const span = readCodeSpan(dt);
    if (!span) return null;
    let chain = "";
    try {
      const hit = await fetchSymbolAt(span.path, span.start, span.end);
      chain = hit.chain?.trim() ?? "";
    } catch {
      chain = "";
    }
    const lines = formatLineSpan(span.start, span.end);
    return chain
      ? symbolMentionSource(span.path, { symbol: chain, lines })
      : symbolMentionSource(span.path, { lines });
  }

  const tree = readTreePaths(dt);
  if (tree) return chips(tree, projectRoot);

  if (dt.files.length > 0) {
    const paths = Array.from(dt.files).map((file) => osFilePath(file) ?? file.name);
    return chips(paths, projectRoot);
  }

  const loose = readLoosePaths(dt);
  if (loose.length > 0) return chips(loose, projectRoot);
  return null;
}

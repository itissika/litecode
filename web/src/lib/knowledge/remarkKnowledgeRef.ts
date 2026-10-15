import {
  humanFileLabel,
  humanSymbolLabel,
  isWorkspaceFileRef,
  parseLineSpan,
  splitBodyRefs,
} from "./markers";

const KNOWLEDGE_SCHEME = "knowledge:";
const LOC_SCHEME = "loc:";

function knowledgeRefHref(key: string): string {
  return `${KNOWLEDGE_SCHEME}${encodeURIComponent(key)}`;
}

export function parseKnowledgeRef(href: string | undefined): string | null {
  if (!href?.startsWith(KNOWLEDGE_SCHEME)) return null;
  try {
    const key = decodeURIComponent(href.slice(KNOWLEDGE_SCHEME.length)).trim();
    return key || null;
  } catch {
    return null;
  }
}

/**
 * A file citation the reply can open. `line` is the start of a line range, when
 * present. `lines` is the raw range attribute, carried only so the chip can find
 * its `ref_key`; a symbol citation carries neither.
 */
function locationHref(
  path: string,
  line?: number | null,
  symbol?: string | null,
  lines?: string | null,
): string {
  const base = `${LOC_SCHEME}${encodeURIComponent(path)}`;
  const chain = symbol?.trim() ?? "";
  const query = chain
    ? `?symbol=${encodeURIComponent(chain)}`
    : lines
      ? `?lines=${encodeURIComponent(lines)}`
      : "";
  const hash = !chain && line != null && line >= 1 ? `#L${line}` : "";
  return `${base}${query}${hash}`;
}

export function parseLocationRef(
  href: string | undefined,
): { path: string; line: number | null; symbol: string | null; lines?: string | null } | null {
  if (!href?.startsWith(LOC_SCHEME)) return null;
  const raw = href.slice(LOC_SCHEME.length);
  const hashAt = raw.indexOf("#");
  const beforeHash = hashAt >= 0 ? raw.slice(0, hashAt) : raw;
  const queryAt = beforeHash.indexOf("?");
  const pathPart = queryAt >= 0 ? beforeHash.slice(0, queryAt) : beforeHash;
  let path: string;
  try {
    path = decodeURIComponent(pathPart);
  } catch {
    return null;
  }
  if (!isWorkspaceFileRef(path)) return null;
  let symbol: string | null = null;
  let lines: string | null = null;
  if (queryAt >= 0) {
    const params = new URLSearchParams(beforeHash.slice(queryAt + 1));
    const chain = params.get("symbol")?.trim() ?? "";
    symbol = chain || null;
    lines = symbol ? null : (params.get("lines") ?? null);
  }
  const extra = lines != null ? { lines } : {};
  const fragment = hashAt >= 0 ? raw.slice(hashAt + 1) : "";
  const lineMatch = /^L(\d+)$/.exec(fragment);
  if (symbol || !lineMatch) return { path, line: null, symbol, ...extra };
  const line = Number(lineMatch[1]);
  if (!Number.isInteger(line) || line < 1) return { path, line: null, symbol, ...extra };
  return { path, line, symbol, ...extra };
}

interface MdastNode {
  type: string;
  value?: string;
  url?: string;
  children?: MdastNode[];
}

function nodesFromText(value: string): MdastNode[] {
  const segments = splitBodyRefs(value);
  if (segments.length === 1 && segments[0]?.type === "text") {
    return [{ type: "text", value }];
  }
  const nodes: MdastNode[] = [];
  for (const segment of segments) {
    if (segment.type === "text") {
      if (segment.value) nodes.push({ type: "text", value: segment.value });
      continue;
    }
    if (segment.type === "ref") {
      nodes.push({
        type: "link",
        url: knowledgeRefHref(segment.id),
        children: [{ type: "text", value: segment.label }],
      });
      continue;
    }
    const symbol = segment.type === "symbol" ? segment.symbol?.trim() ?? "" : "";
    // Only a line range opens at a line; a symbol citation never uses lines.
    const line =
      segment.type === "symbol" && !symbol && segment.lines
        ? (parseLineSpan(segment.lines)?.start ?? null)
        : null;
    if (segment.label && isWorkspaceFileRef(segment.path)) {
      nodes.push({
        type: "link",
        url: locationHref(segment.path, line, symbol, symbol ? null : segment.rawLines),
        children: [
          {
            type: "text",
            value: symbol ? humanSymbolLabel(segment.path, symbol) : humanFileLabel(segment.path),
          },
        ],
      });
      continue;
    }
    if (segment.label) nodes.push({ type: "text", value: segment.label });
  }
  return nodes;
}

function walk(node: MdastNode): void {
  if (!node.children) return;
  const next: MdastNode[] = [];
  for (const child of node.children) {
    if (
      child.type === "text" &&
      typeof child.value === "string" &&
      node.type !== "link"
    ) {
      next.push(...nodesFromText(child.value));
      continue;
    }
    if (child.type !== "code" && child.type !== "inlineCode") walk(child);
    next.push(child);
  }
  node.children = next;
}

/**
 * Turn a citation in prose into what a person reads.
 * A node becomes a `knowledge:` link whose text is the key.
 * A file or symbol becomes a `loc:` link whose text is the short capsule.
 * Code and inline code are separate AST nodes, so they are left alone.
 */
export function remarkKnowledgeRef() {
  return (tree: unknown) => {
    if (!tree || typeof tree !== "object") return;
    walk(tree as MdastNode);
  };
}

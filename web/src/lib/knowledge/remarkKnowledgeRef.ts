import { splitKnowledgeRefs } from "./markers";

const KNOWLEDGE_SCHEME = "knowledge:";

export function knowledgeRefHref(key: string): string {
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

interface MdastNode {
  type: string;
  value?: string;
  url?: string;
  children?: MdastNode[];
}

function nodesFromText(value: string): MdastNode[] {
  const segments = splitKnowledgeRefs(value);
  if (segments.length === 1 && segments[0]?.type === "text") {
    return [{ type: "text", value }];
  }
  const nodes: MdastNode[] = [];
  for (const segment of segments) {
    if (segment.type === "text") {
      if (segment.value) nodes.push({ type: "text", value: segment.value });
      continue;
    }
    nodes.push({
      type: "link",
      url: knowledgeRefHref(segment.id),
      children: [{ type: "text", value: segment.label }],
    });
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
 * Turn a Mention shortcode in prose into a `knowledge:` link.
 * The link target is `id`. The link text is `label`.
 * Code and inline code are separate AST nodes, so they are left alone.
 */
export function remarkKnowledgeRef() {
  return (tree: unknown) => {
    if (!tree || typeof tree !== "object") return;
    walk(tree as MdastNode);
  };
}

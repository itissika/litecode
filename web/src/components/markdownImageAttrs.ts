import { $remark } from "@milkdown/kit/utils";

type MarkdownNode = {
  type?: string;
  title?: unknown;
  url?: unknown;
  alt?: unknown;
  children?: MarkdownNode[];
};

/**
 * mdast leaves `title` and `alt` as null when the markdown omits them.
 * image-block validates `caption` (copied from `title`) as a string, so a
 * bare `![alt](src)` throws while the editor is created.
 */
export function fillMarkdownImageAttrs(node: MarkdownNode): void {
  if (node.type === "image" || node.type === "image-block") {
    if (typeof node.title !== "string") node.title = "";
    if (typeof node.url !== "string") node.url = "";
    if (node.type === "image" && typeof node.alt !== "string") node.alt = "";
  }
  for (const child of node.children ?? []) fillMarkdownImageAttrs(child);
}

export const remarkImageStringAttrs = $remark(
  "remark-image-string-attrs",
  () => () => (tree: MarkdownNode) => {
    fillMarkdownImageAttrs(tree);
  },
);

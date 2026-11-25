import type { JSONContent } from "@tiptap/core";
import type { MentionOptions } from "@tiptap/extension-mention";

import { citationMarkdown, splitBodyRefs } from "../../lib/knowledge/markers";

/** Single TipTap Mention configuration: serializes every chip as a standard Markdown link. */
export function citationMentionOptions(
  suggestion: MentionOptions["suggestion"],
): Partial<MentionOptions> {
  return {
    HTMLAttributes: { class: "mention-chip" },
    deleteTriggerWithBackspace: true,
    renderText({ node }) {
      const path = String(node.attrs.id ?? "");
      const label = node.attrs.label == null ? null : String(node.attrs.label);
      const symbol = String(node.attrs.symbol ?? "").trim() || null;
      const lines = node.attrs.lines == null ? null : String(node.attrs.lines);
      const badLines = node.attrs.badLines == null ? null : String(node.attrs.badLines);
      return citationMarkdown({ path, label, symbol, lines, badLines });
    },
    suggestion,
  };
}

/** Turn stored body text into a TipTap document. Workspace Markdown links become mention nodes. */
export function bodyToContent(text: string): JSONContent {
  const lines = text.length > 0 ? text.split(/\r?\n/) : [""];
  return {
    type: "doc",
    content: lines.map((line) => {
      const content = inlineContent(line);
      return content.length > 0 ? { type: "paragraph", content } : { type: "paragraph" };
    }),
  };
}

function inlineContent(line: string): JSONContent[] {
  const nodes: JSONContent[] = [];
  for (const segment of splitBodyRefs(line)) {
    if (segment.type === "text") {
      if (segment.value) nodes.push({ type: "text", text: segment.value });
      continue;
    }
    nodes.push({
      type: "mention",
      attrs: {
        id: segment.path,
        label: segment.label,
        symbol: segment.symbol,
        lines: segment.lines,
        badLines: segment.badLines,
        mentionSuggestionChar: "@",
      },
    });
  }
  return nodes;
}

/** Inline nodes for one citation line, ready for `insertContentAt`. */
export function mentionInlineContent(text: string): JSONContent[] {
  const paragraph = bodyToContent(text).content?.[0];
  return paragraph?.content ?? [];
}

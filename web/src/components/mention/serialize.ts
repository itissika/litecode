import type { JSONContent } from "@tiptap/core";
import type { MentionOptions } from "@tiptap/extension-mention";

import { fileMentionSource, mentionSource, splitBodyRefs, symbolMentionSource } from "../../lib/knowledge/markers";

export function knowledgeMentionOptions(
  suggestion: MentionOptions["suggestion"],
): Partial<MentionOptions> {
  return {
    HTMLAttributes: { class: "mention-chip" },
    deleteTriggerWithBackspace: true,
    renderText({ node }) {
      const id = String(node.attrs.id ?? "");
      const label = String(node.attrs.label ?? id);
      return mentionSource(id, label);
    },
    suggestion,
  };
}

export function fileMentionOptions(
  suggestion: MentionOptions["suggestion"],
): Partial<MentionOptions> {
  return {
    HTMLAttributes: { class: "mention-chip" },
    deleteTriggerWithBackspace: true,
    renderText({ node }) {
      const path = String(node.attrs.id ?? "");
      const label = String(node.attrs.label ?? path);
      const symbol = String(node.attrs.symbol ?? "").trim();
      const lines = String(node.attrs.lines ?? "").trim();
      if (symbol || lines) {
        return symbolMentionSource(path, {
          symbol: symbol || undefined,
          lines: lines || undefined,
          label,
        });
      }
      return fileMentionSource(path, label);
    },
    suggestion,
  };
}

/** Turn stored body text into a TipTap document. Valid shortcodes become mention nodes. */
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
    if (segment.type === "file" || segment.type === "symbol") {
      nodes.push({
        type: "fileMention",
        attrs: {
          id: segment.path,
          label: segment.label,
          symbol: segment.type === "symbol" ? segment.symbol : null,
          lines: segment.type === "symbol" ? segment.lines : null,
          mentionSuggestionChar: "/",
        },
      });
      continue;
    }
    nodes.push({
      type: "mention",
      attrs: {
        id: segment.id,
        label: segment.label,
        mentionSuggestionChar: "@",
      },
    });
  }
  return nodes;
}

/** Inline nodes for one shortcode line, ready for `insertContentAt`. */
export function mentionInlineContent(text: string): JSONContent[] {
  const paragraph = bodyToContent(text).content?.[0];
  return paragraph?.content ?? [];
}

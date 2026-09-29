import type { JSONContent } from "@tiptap/core";
import type { MentionOptions } from "@tiptap/extension-mention";

import { fileMentionSource, mentionSource, splitBodyRefs } from "./markers";

export function knowledgeMentionOptions(
  suggestion: MentionOptions["suggestion"],
): Partial<MentionOptions> {
  return {
    HTMLAttributes: { class: "knowledge-token" },
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
    HTMLAttributes: { class: "knowledge-token is-file" },
    deleteTriggerWithBackspace: true,
    renderText({ node }) {
      const path = String(node.attrs.id ?? "");
      const label = String(node.attrs.label ?? path);
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
    if (segment.type === "file") {
      nodes.push({
        type: "fileMention",
        attrs: {
          id: segment.path,
          label: segment.label,
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

import { Editor } from "@tiptap/core";
import Document from "@tiptap/extension-document";
import Mention from "@tiptap/extension-mention";
import Paragraph from "@tiptap/extension-paragraph";
import Text from "@tiptap/extension-text";
import { describe, expect, it } from "vitest";

import { mentionSource } from "./markers";
import { bodyToContent, knowledgeMentionOptions } from "./mentionDoc";

function textOf(source: string) {
  const editor = new Editor({
    element: document.createElement("div"),
    extensions: [
      Document,
      Paragraph,
      Text,
      Mention.configure(knowledgeMentionOptions({ char: "@" })),
    ],
    content: bodyToContent(source),
  });
  const text = editor.getText({ blockSeparator: "\n" });
  editor.destroy();
  return text;
}

describe("bodyToContent", () => {
  it("round-trips prose and a shortcode through the editor text", () => {
    const source = `见 ${mentionSource("seq")}。\n下一行`;
    expect(textOf(source)).toBe(source);
  });

  it("leaves a shortcode whose id is not a key as text", () => {
    const source = '[@ id="bad/key" label="bad/key"]';
    expect(textOf(source)).toBe(source);
  });

  it("keeps an empty body empty", () => {
    expect(textOf("")).toBe("");
  });
});

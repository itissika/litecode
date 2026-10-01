import { Editor } from "@tiptap/core";
import Document from "@tiptap/extension-document";
import Mention from "@tiptap/extension-mention";
import Paragraph from "@tiptap/extension-paragraph";
import Text from "@tiptap/extension-text";
import { describe, expect, it } from "vitest";

import { fileMentionSource, mentionSource, symbolMentionSource } from "./markers";
import { bodyToContent, fileMentionOptions, knowledgeMentionOptions } from "./mentionDoc";

function textOf(source: string) {
  const editor = new Editor({
    element: document.createElement("div"),
    extensions: [
      Document,
      Paragraph,
      Text,
      Mention.configure(knowledgeMentionOptions({ char: "@" })),
      Mention.extend({
        name: "fileMention",
        addAttributes() {
          return {
            ...this.parent?.(),
            symbol: { default: null },
            lines: { default: null },
          };
        },
      }).configure(fileMentionOptions({ char: "/" })),
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

  it("round-trips a file citation without turning it into a node citation", () => {
    const source = `见 ${mentionSource("seq")} 与 ${fileMentionSource("src/a.rs")}`;
    expect(textOf(source)).toBe(source);
  });

  it("round-trips a symbol citation and a line range", () => {
    const symbol = symbolMentionSource("src/session/manager.rs", {
      symbol: "impl SessionManager › fn append_reminder",
      lines: "2148-2165",
      label: "fn append_reminder",
    });
    const range = symbolMentionSource("src/a.rs", { lines: "4", label: "a.rs" });
    const source = `${symbol} ${range}`;
    expect(textOf(source)).toBe(source);
  });

  it("keeps an empty body empty", () => {
    expect(textOf("")).toBe("");
  });
});

import { Editor } from "@tiptap/core";
import Document from "@tiptap/extension-document";
import Mention from "@tiptap/extension-mention";
import Paragraph from "@tiptap/extension-paragraph";
import Text from "@tiptap/extension-text";
import { describe, expect, it } from "vitest";

import { bodyToContent, citationMentionOptions } from "./mentionDoc";

function textOf(source: string) {
  const editor = new Editor({
    element: document.createElement("div"),
    extensions: [
      Document,
      Paragraph,
      Text,
      Mention.extend({
        addAttributes() {
          return {
            ...this.parent?.(),
            symbol: { default: null },
            lines: { default: null },
            badLines: { default: null },
          };
        },
      }).configure(citationMentionOptions({ char: "@" })),
    ],
    content: bodyToContent(source),
  });
  const text = editor.getText({ blockSeparator: "\n" });
  editor.destroy();
  return text;
}

describe("bodyToContent", () => {
  it("round-trips prose and a knowledge link through the editor text", () => {
    const source = "见 [seq](knowledge/内核/seq.md)。\n下一行";
    expect(textOf(source)).toBe(source);
  });

  it("leaves external web links as plain text", () => {
    const source = "见 [docs](https://example.com/a)";
    expect(textOf(source)).toBe(source);
  });

  it("round-trips file, symbol, and line range citations", () => {
    const source =
      "见 [seq](knowledge/seq.md) 与 [a.rs](src/a.rs) 与 [Store.save](src/a.rs#Store.save) 与 [a.rs:4-9](src/a.rs#L4-L9)";
    expect(textOf(source)).toBe(source);
  });

  it("preserves malformed line anchors on round-trip so backend can flag them", () => {
    const source = "[a.rs](src/a.rs#12-30)";
    expect(textOf(source)).toBe(source);
  });

  it("keeps an empty body empty", () => {
    expect(textOf("")).toBe("");
  });
});

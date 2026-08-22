import { describe, expect, it } from "vitest";

import {
  isWysiwygMarkdownPath,
  resolveMdEditorView,
  WYSIWYG_MARKDOWN_MAX_CHARS,
} from "./wysiwygMarkdown";

describe("isWysiwygMarkdownPath", () => {
  it("accepts .md and rejects .mdx", () => {
    expect(isWysiwygMarkdownPath("README.md")).toBe(true);
    expect(isWysiwygMarkdownPath("docs/Guide.MD")).toBe(true);
    expect(isWysiwygMarkdownPath("page.mdx")).toBe(false);
    expect(isWysiwygMarkdownPath("src/main.ts")).toBe(false);
  });
});

describe("resolveMdEditorView", () => {
  it("defaults .md to wysiwyg", () => {
    expect(resolveMdEditorView("notes.md", 10, undefined)).toBe("wysiwyg");
  });

  it("honors an explicit source override", () => {
    expect(resolveMdEditorView("notes.md", 10, "source")).toBe("source");
  });

  it("forces source for huge files even if override is wysiwyg", () => {
    expect(
      resolveMdEditorView(
        "notes.md",
        WYSIWYG_MARKDOWN_MAX_CHARS + 1,
        "wysiwyg",
      ),
    ).toBe("source");
  });

  it("never uses wysiwyg for non-md", () => {
    expect(resolveMdEditorView("a.ts", 10, "wysiwyg")).toBe("source");
  });
});

import { describe, expect, it } from "vitest";

import {
  isPreviewMarkdownPath,
  PREVIEW_MARKDOWN_MAX_CHARS,
  resolveMdEditorView,
} from "./markdownView";

describe("isPreviewMarkdownPath", () => {
  it("accepts .md and rejects .mdx", () => {
    expect(isPreviewMarkdownPath("README.md")).toBe(true);
    expect(isPreviewMarkdownPath("docs/Guide.MD")).toBe(true);
    expect(isPreviewMarkdownPath("page.mdx")).toBe(false);
    expect(isPreviewMarkdownPath("src/main.ts")).toBe(false);
  });
});

describe("resolveMdEditorView", () => {
  it("defaults .md to preview", () => {
    expect(resolveMdEditorView("notes.md", 10, undefined)).toBe("preview");
  });

  it("honors an explicit source override", () => {
    expect(resolveMdEditorView("notes.md", 10, "source")).toBe("source");
  });

  it("forces source for huge files even if override is preview", () => {
    expect(
      resolveMdEditorView(
        "notes.md",
        PREVIEW_MARKDOWN_MAX_CHARS + 1,
        "preview",
      ),
    ).toBe("source");
  });

  it("never uses preview for non-md", () => {
    expect(resolveMdEditorView("a.ts", 10, "preview")).toBe("source");
  });
});

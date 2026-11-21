import { act, cleanup, render } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { getMarkdownHighlighter } from "../lib/shiki";
import { CodeBlock } from "./CodeBlock";

vi.mock("../lib/shiki", () => ({
  getMarkdownHighlighter: vi.fn(async () => ({
    codeToHtml: (code: string) => '<span class="hl">' + code + "</span>",
  })),
  isSupportedHighlightLang: () => true,
  normalizeLang: (lang: string) => lang,
  SHIKI_THEME_DARK: "dark-plus",
  SHIKI_THEME_LIGHT: "min-light",
}));

afterEach(() => {
  cleanup();
  vi.mocked(getMarkdownHighlighter).mockClear();
});

async function settle() {
  await act(async () => {
    await Promise.resolve();
  });
}

describe("CodeBlock while the stream is live", () => {
  it("does not run the highlighter for a code block that is still streaming", async () => {
    const { container } = render(
      <CodeBlock code="const a = 1;" lang="typescript" streaming />,
    );
    await settle();

    expect(getMarkdownHighlighter).not.toHaveBeenCalled();
    expect(container.querySelector(".shiki-host")).toBeNull();
    expect(container.querySelector("pre")).not.toBeNull();
  });

  it("never highlights a growing fence mid-stream", async () => {
    const view = render(
      <CodeBlock code="const a = 1;" lang="typescript" streaming />,
    );
    for (let i = 0; i < 8; i += 1) {
      view.rerender(
        <CodeBlock code={"const a = 1; // " + i} lang="typescript" streaming />,
      );
      await settle();
    }

    expect(getMarkdownHighlighter).not.toHaveBeenCalled();
  });

  it("highlights exactly once when the stream seals", async () => {
    const view = render(
      <CodeBlock code="const a = 1;" lang="typescript" streaming />,
    );
    await settle();
    expect(getMarkdownHighlighter).not.toHaveBeenCalled();

    view.rerender(
      <CodeBlock code="const a = 1;" lang="typescript" streaming={false} />,
    );
    await settle();

    expect(getMarkdownHighlighter).toHaveBeenCalledTimes(1);
    expect(view.container.querySelector(".shiki-host")).not.toBeNull();
  });

  it("still highlights a sealed code block on mount", async () => {
    const view = render(<CodeBlock code="const a = 1;" lang="typescript" />);
    await settle();

    expect(getMarkdownHighlighter).toHaveBeenCalledTimes(1);
    expect(view.container.querySelector(".shiki-host")).not.toBeNull();
  });
});

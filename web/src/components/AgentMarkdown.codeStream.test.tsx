import { act, cleanup, render } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { getMarkdownHighlighter } from "../lib/shiki";
import { AgentMarkdown } from "./AgentMarkdown";

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

const CLOSED_FENCE = "\u0060\u0060\u0060typescript\nconst a = 1;\n\u0060\u0060\u0060";
const OPEN_TAIL = "\u0060\u0060\u0060json\n{\"a\": 1}\n\u0060\u0060\u0060";

describe("streaming markdown only streams its tail block", () => {
  it("highlights the closed fence once and leaves the streaming tail plain", async () => {
    const view = render(
      <AgentMarkdown
        text={CLOSED_FENCE + "\n\n" + OPEN_TAIL}
        streaming
      />,
    );
    await settle();

    expect(getMarkdownHighlighter).toHaveBeenCalledTimes(1);
    expect(view.container.querySelectorAll(".shiki-host")).toHaveLength(1);
  });

  it("highlights the tail once the stream seals", async () => {
    const view = render(
      <AgentMarkdown
        text={CLOSED_FENCE + "\n\n" + OPEN_TAIL}
        streaming
      />,
    );
    await settle();

    view.rerender(
      <AgentMarkdown
        text={CLOSED_FENCE + "\n\n" + OPEN_TAIL}
        streaming={false}
      />,
    );
    await settle();

    expect(getMarkdownHighlighter).toHaveBeenCalledTimes(2);
    expect(view.container.querySelectorAll(".shiki-host")).toHaveLength(2);
  });
});

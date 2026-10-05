import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { resolveCitations } from "../api/workspace";
import { resetCitationCacheForTests } from "../lib/citationResolve";
import { fileMentionSource, mentionSource, symbolMentionSource } from "../lib/knowledge/markers";
import { useEditorStore } from "../stores/editorStore";
import { useKnowledgeStore } from "../stores/knowledgeStore";
import { useSessionStore } from "../stores/sessionStore";
import { AgentMarkdown } from "./AgentMarkdown";

const originalOpenFile = useEditorStore.getState().openFile;
const originalOpenFileAt = useEditorStore.getState().openFileAt;

vi.mock("../api/workspace", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../api/workspace")>();
  return {
    ...actual,
    resolveCitations: vi.fn(),
  };
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  resetCitationCacheForTests();
  useSessionStore.setState({ project: "" });
  useEditorStore.setState({
    openFile: originalOpenFile,
    openFileAt: originalOpenFileAt,
  } as never);
  useKnowledgeStore.setState({
    byId: new Map(),
    byKey: new Map(),
    focusedId: null,
  });
  vi.mocked(resolveCitations).mockReset();
});

describe("AgentMarkdown raw HTML boundary", () => {
  it("reveals a small stream one character at a time", () => {
    vi.useFakeTimers();
    const { container, rerender } = render(<AgentMarkdown text="" streaming />);

    rerender(<AgentMarkdown text="abc" streaming />);
    act(() => vi.advanceTimersByTime(28));

    expect(container.textContent).toBe("a");
  });

  it("accelerates when the visual stream falls behind", () => {
    vi.useFakeTimers();
    const { container, rerender } = render(<AgentMarkdown text="" streaming />);

    rerender(<AgentMarkdown text={"a".repeat(300)} streaming />);
    act(() => vi.advanceTimersByTime(28));

    expect(container.textContent).toHaveLength(5);
  });

  it("renders the complete text when a stream finishes between renders", () => {
    const { container, rerender } = render(<AgentMarkdown text="" streaming />);

    rerender(
      <AgentMarkdown text="complete assistant response" streaming={false} />,
    );

    expect(container.textContent).toContain("complete assistant response");
  });

  it("renders raw HTML as inert text instead of executable DOM", () => {
    const raw = [
      '<iframe srcdoc="<script>globalThis.pwned = true</script>"></iframe>',
      "<script>globalThis.pwned = true</script>",
      '<img src="x" onerror="globalThis.pwned = true">',
      '<div style="background:url(javascript:alert(1))">styled</div>',
      '<meta http-equiv="refresh" content="0;url=https://attacker.example">',
    ].join("\n\n");

    const { container } = render(<AgentMarkdown text={raw} />);

    expect(container.querySelector("iframe")).toBeNull();
    expect(container.querySelector("script")).toBeNull();
    expect(container.querySelector("img")).toBeNull();
    expect(container.querySelector("[style]")).toBeNull();
    expect(container.querySelector("meta")).toBeNull();
    expect(container.textContent).toContain("<iframe");
    expect(container.textContent).toContain("<script>");
    expect(container.textContent).toContain("onerror=");
    expect(container.textContent).toContain("<meta");
  });

  it("keeps safe Markdown and GFM rendering", () => {
    const { container } = render(
      <AgentMarkdown
        text={[
          "## Safe heading",
          "",
          "**bold** and [docs](https://example.com)",
          "",
          "- [x] shipped",
          "",
          "| Name | Value |",
          "| --- | --- |",
          "| safe | yes |",
        ].join("\n")}
      />,
    );

    expect(screen.getByRole("heading", { name: "Safe heading" })).toBeTruthy();
    expect(screen.getByText("bold").tagName).toBe("STRONG");
    expect(
      screen.getByRole("link", { name: "docs" }).getAttribute("href"),
    ).toBe("https://example.com");
    expect(
      container.querySelector('input[type="checkbox"][checked]'),
    ).not.toBeNull();
    expect(screen.getByRole("table")).toBeTruthy();
  });
});

describe("AgentMarkdown citations", () => {
  it("leaves file links as text when citations are off", async () => {
    useSessionStore.setState({ project: "E:/ws" });
    const { container } = render(
      <AgentMarkdown text="See [validate.ts](file:src/a.ts) here" />,
    );
    expect(screen.queryByRole("button")).toBeNull();
    expect(screen.queryByRole("link")).toBeNull();
    expect(container.textContent).toContain("validate.ts");
    await act(async () => {
      await Promise.resolve();
    });
    expect(resolveCitations).not.toHaveBeenCalled();
  });

  it("does not query an illegal path or a path inside a code block", async () => {
    useSessionStore.setState({ project: "E:/ws" });
    render(
      <AgentMarkdown
        citations
        text={"[secret](file:../secret)\n\n```\n[a](file:src/a.ts)\n```"}
      />,
    );
    expect(screen.getByText("secret")).toBeTruthy();
    expect(screen.queryByRole("button")).toBeNull();
    await act(async () => {
      await Promise.resolve();
    });
    expect(resolveCitations).not.toHaveBeenCalled();
  });

  it("opens a file shortcode at the start line and a markdown file link as a chip", () => {
    const openFile = vi.fn(async () => {});
    const openFileAt = vi.fn(async () => {});
    useEditorStore.setState({ openFile, openFileAt } as never);
    const range = symbolMentionSource("src/a.rs", { lines: "4-9" });
    const { container } = render(
      <AgentMarkdown
        citations
        text={`See ${fileMentionSource("src/a.rs")} and ${range} and [validate.ts](file:src/auth/validate.ts)`}
      />,
    );

    expect(container.textContent).toContain(".../src/a.rs");
    expect(container.textContent).not.toContain("src/a.rs : 4-9");
    expect(container.textContent).toContain(".../auth/validate.ts");
    expect(container.textContent).not.toContain('[@ file="src/a.rs"]');
    expect(resolveCitations).not.toHaveBeenCalled();

    const files = screen.getAllByRole("button", { name: ".../src/a.rs" });
    expect(files).toHaveLength(2);
    expect(files[0]?.parentElement?.className).toContain("knowledge-token");
    expect(files[0]?.parentElement?.className).toContain("is-file");
    expect(files[0]?.parentElement?.className).not.toContain("is-symbol");
    expect(files[1]?.parentElement?.className).not.toContain("is-symbol");
    fireEvent.click(files[0]!);
    expect(openFile).toHaveBeenCalledWith("src/a.rs");
    fireEvent.click(files[1]!);
    expect(openFileAt).toHaveBeenCalledWith("src/a.rs", 4);

    fireEvent.click(screen.getByRole("button", { name: ".../auth/validate.ts" }));
    expect(openFile).toHaveBeenCalledWith("src/auth/validate.ts");
  });

  it("opens a relative file link at its line or symbol", () => {
    const openFile = vi.fn(async () => {});
    const openFileAt = vi.fn(async () => {});
    useEditorStore.setState({ openFile, openFileAt } as never);
    render(
      <AgentMarkdown
        citations
        text={"See [validate](src/auth/validate.ts#L42) and [user](src/session.rs#Session.user)"}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: ".../auth/validate.ts" }));
    expect(openFileAt).toHaveBeenCalledWith("src/auth/validate.ts", 42);
    fireEvent.click(screen.getByRole("button", { name: "session.rs : Session.user" }));
    expect(openFile).toHaveBeenCalledWith("src/session.rs");
  });

  it("renders a file-ish link we cannot open as plain text", () => {
    const { container } = render(
      <AgentMarkdown citations text={"[secret](../secret) and [hosts](/etc/hosts)"} />,
    );

    expect(container.textContent).toContain("secret");
    expect(container.textContent).toContain("hosts");
    expect(screen.queryByRole("link")).toBeNull();
    expect(screen.queryByRole("button")).toBeNull();
  });

  it("does not turn a markdown file link inside inline code into a chip", () => {
    render(<AgentMarkdown citations text={"`[a](src/a.ts)`"} />);
    expect(screen.queryByRole("button")).toBeNull();
  });

  it("opens a symbol citation at the file, and at the start line when a range is written", () => {
    const openFile = vi.fn(async () => {});
    const openFileAt = vi.fn(async () => {});
    useEditorStore.setState({ openFile, openFileAt } as never);
    const symbol = symbolMentionSource("src/a.rs", {
      symbol: "impl Store › fn save",
    });
    const ranged = symbolMentionSource("src/a.rs", {
      symbol: "impl Store › fn save",
      lines: "2148-2165",
    });
    const { container } = render(
      <AgentMarkdown citations text={`${symbol} ${ranged}`} />,
    );
    expect(container.textContent).toContain("a.rs : impl Store › fn save");
    expect(container.textContent).not.toContain("2148-2165");
    const symbolChips = screen.getAllByRole("button", {
      name: "a.rs : impl Store › fn save",
    });
    expect(symbolChips).toHaveLength(2);
    expect(symbolChips[0]?.parentElement?.className).toContain("is-symbol");
    fireEvent.click(symbolChips[0]!);
    expect(openFile).toHaveBeenCalledWith("src/a.rs");
    fireEvent.click(symbolChips[1]!);
    expect(openFileAt).toHaveBeenCalledWith("src/a.rs", 2148);
  });

  it("renders a knowledge node shortcode as a capsule beside a file citation", () => {
    useKnowledgeStore.setState({
      byKey: new Map([["seq", { id: "seq", key: "seq" }]]),
      byId: new Map([["seq", { id: "seq", key: "seq" }]]),
    } as never);
    const fileShortcode = fileMentionSource("src/a.rs");
    const { container } = render(
      <AgentMarkdown
        citations
        text={`See ${mentionSource("seq", "序号")} and [a.ts](file:src/a.ts) plus ${fileShortcode}`}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "seq" }));
    expect(useKnowledgeStore.getState().focusedId).toBe("seq");
    expect(screen.getByRole("button", { name: ".../src/a.rs" }).parentElement?.className).toContain(
      "is-file",
    );
    expect(screen.getAllByRole("button")).toHaveLength(3);
    expect(container.textContent).toContain("a.ts");
    expect(container.textContent).not.toContain(fileShortcode);
  });

  it("leaves a node shortcode as text when citations are off, and marks an unknown key", () => {
    const { rerender, container } = render(
      <AgentMarkdown text={`see ${mentionSource("seq")}`} />,
    );
    expect(screen.queryByRole("button")).toBeNull();
    expect(container.textContent).toContain('[@ key="seq"]');

    rerender(<AgentMarkdown citations text={`see ${mentionSource("missing")}`} />);
    expect(screen.queryByRole("button")).toBeNull();
    expect(container.textContent).toContain("missing");
    expect(container.querySelector(".knowledge-token.is-invalid")).not.toBeNull();
  });

  it("does not turn a node shortcode inside a code block into a capsule", () => {
    useKnowledgeStore.setState({
      byKey: new Map([["seq", { id: "seq", key: "seq" }]]),
      byId: new Map([["seq", { id: "seq", key: "seq" }]]),
    } as never);
    render(
      <AgentMarkdown citations text={`\`\`\`\n${mentionSource("seq")}\n\`\`\``} />,
    );
    expect(screen.queryByRole("button", { name: "seq" })).toBeNull();
  });

  it("renders a web link as an ordinary link", () => {
    render(<AgentMarkdown citations text="[Docs](https://example.com/docs)" />);
    const link = screen.getByRole("link", { name: "Docs" });
    expect(link.getAttribute("href")).toBe("https://example.com/docs");
    expect(link.getAttribute("target")).toBe("_blank");
    expect(link.className).not.toContain("agent-citation");
    expect(resolveCitations).not.toHaveBeenCalled();
  });

  it("does not open a file citation written inside a code block", () => {
    render(
      <AgentMarkdown
        citations
        text={`\`\`\`\n${fileMentionSource("src/a.rs")}\n\`\`\``}
      />,
    );
    expect(screen.queryByRole("button", { name: ".../src/a.rs" })).toBeNull();
  });
});

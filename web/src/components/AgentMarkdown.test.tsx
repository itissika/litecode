import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { resolveCitations } from "../api/workspace";
import { resetCitationCacheForTests } from "../lib/citationResolve";
import { useEditorStore } from "../stores/editorStore";
import { useSessionStore } from "../stores/sessionStore";
import { AgentMarkdown } from "./AgentMarkdown";

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
  useEditorStore.setState({ openFileAt: originalOpenFileAt } as never);
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

  it("renders a button only after the file is confirmed", async () => {
    useSessionStore.setState({ project: "E:/ws" });
    const openFileAt = vi.fn(async () => {});
    useEditorStore.setState({ openFileAt } as never);
    vi.mocked(resolveCitations).mockResolvedValue([
      { exists: true, path: "src/auth/validate.ts", line: 42 },
    ]);

    render(
      <AgentMarkdown
        citations
        text="See [validate.ts:42](file:src/auth/validate.ts#L42)"
      />,
    );

    const button = await screen.findByRole("button", {
      name: "Open src/auth/validate.ts:42",
    });
    fireEvent.click(button);
    expect(openFileAt).toHaveBeenCalledWith("src/auth/validate.ts", 42);
  });

  it("keeps the label as text when the file is missing", async () => {
    useSessionStore.setState({ project: "E:/ws" });
    vi.mocked(resolveCitations).mockResolvedValue([{ exists: false }]);
    const { container } = render(
      <AgentMarkdown citations text="See [missing.ts](file:src/missing.ts)" />,
    );
    await waitFor(() => expect(resolveCitations).toHaveBeenCalled());
    expect(screen.queryByRole("button")).toBeNull();
    expect(container.textContent).toContain("missing.ts");
  });

  it("renders a web citation without checking that the page exists", () => {
    render(<AgentMarkdown citations text="[Docs](https://example.com/docs)" />);
    const link = screen.getByRole("link", { name: "Docs" });
    expect(link.getAttribute("href")).toBe("https://example.com/docs");
    expect(link.getAttribute("target")).toBe("_blank");
    expect(link.className).toContain("agent-citation");
    expect(resolveCitations).not.toHaveBeenCalled();
  });

  it("keeps a cached hit when streaming ends", async () => {
    useSessionStore.setState({ project: "E:/ws" });
    vi.mocked(resolveCitations).mockResolvedValue([
      { exists: true, path: "src/a.ts" },
    ]);
    const { rerender } = render(
      <AgentMarkdown citations streaming text="[a](file:src/a.ts)" />,
    );
    await screen.findByRole("button", { name: "Open src/a.ts" });
    rerender(
      <AgentMarkdown citations streaming={false} text="[a](file:src/a.ts)" />,
    );
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 20));
    });
    expect(resolveCitations).toHaveBeenCalledTimes(1);
  });

  it("asks again when a miss finishes streaming", async () => {
    useSessionStore.setState({ project: "E:/ws" });
    vi.mocked(resolveCitations).mockResolvedValue([{ exists: false }]);
    const { rerender } = render(
      <AgentMarkdown citations streaming text="[a](file:src/a.ts)" />,
    );
    await waitFor(() => expect(resolveCitations).toHaveBeenCalledTimes(1));
    rerender(
      <AgentMarkdown citations streaming={false} text="[a](file:src/a.ts)" />,
    );
    await waitFor(() => expect(resolveCitations).toHaveBeenCalledTimes(2));
  });
});

import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { knowledgeFixture, knowledgeFolderFixture } from "../../lib/knowledge/fixture";
import { fileMentionSource, mentionSource, symbolMentionSource } from "../../lib/knowledge/markers";
import { useEditorStore } from "../../stores/editorStore";
import { knowledgeSnapshot, useKnowledgeStore } from "../../stores/knowledgeStore";
import { KnowledgeMarkdown } from "./KnowledgeMarkdown";

beforeEach(() => {
  useKnowledgeStore.setState({
    ...knowledgeSnapshot(knowledgeFixture, knowledgeFolderFixture),
    expanded: new Set(),
    graphExpanded: new Set(),
    focusedId: null,
    focusNonce: 0,
    flashId: null,
    flashNonce: 0,
    loading: false,
    error: null,
  });
});

afterEach(() => {
  cleanup();
});

describe("KnowledgeMarkdown", () => {
  it("jumps to a registered citation", () => {
    render(<KnowledgeMarkdown sourceId="session" text={`see ${mentionSource("seq")}`} />);
    fireEvent.click(screen.getByRole("button", { name: "seq" }));
    expect(useKnowledgeStore.getState().focusedId).toBe("seq");
    expect(useKnowledgeStore.getState().expanded.has("seq")).toBe(false);
  });

  it("marks an unknown key and does not focus", () => {
    render(
      <KnowledgeMarkdown sourceId="broken-marker" text={mentionSource("not-a-node")} />,
    );
    expect(screen.queryByRole("button", { name: "not-a-node" })).toBeNull();
    expect(
      screen.getByText("not-a-node").closest(".knowledge-token")?.classList.contains("is-invalid"),
    ).toBe(true);
    expect(useKnowledgeStore.getState().focusedId).toBeNull();
  });

  it("does not turn a marker inside a code block into a citation", () => {
    render(
      <KnowledgeMarkdown sourceId="session" text={`\`\`\`\n${mentionSource("seq")}\n\`\`\``} />,
    );
    expect(screen.queryByRole("button", { name: "seq" })).toBeNull();
  });

  it("draws a file citation as the same capsule a person types", () => {
    const openFile = vi.fn(async () => {});
    const openFileAt = vi.fn(async () => {});
    useEditorStore.setState({ openFile, openFileAt } as never);
    render(
      <KnowledgeMarkdown
        sourceId="session"
        text={`see ${fileMentionSource("src/a.rs")} and ${symbolMentionSource("src/a.rs", { symbol: "fn save" })}`}
      />,
    );
    const file = screen.getByRole("button", { name: ".../src/a.rs" });
    expect(file.parentElement?.className).toContain("knowledge-token");
    expect(file.parentElement?.className).toContain("is-file");
    expect(file.parentElement?.className).not.toContain("is-symbol");
    fireEvent.click(file);
    expect(openFile).toHaveBeenCalledWith("src/a.rs");
    const symbol = screen.getByRole("button", { name: "a.rs : fn save" });
    expect(symbol.parentElement?.className).toContain("is-symbol");
    fireEvent.click(symbol);
    expect(openFile).toHaveBeenCalledTimes(2);
  });

  it("does not treat a bare double-bracket as a citation", () => {
    render(<KnowledgeMarkdown sourceId="session" text="see [[seq]] and @seq" />);
    expect(screen.queryByRole("button", { name: "seq" })).toBeNull();
  });
});

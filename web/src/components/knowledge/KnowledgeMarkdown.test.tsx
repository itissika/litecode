import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { knowledgeFixture, knowledgeFolderFixture } from "../../test/fixtures/knowledge";
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
  it("jumps to a valid knowledge node cited by path", () => {
    render(
      <KnowledgeMarkdown
        sourceId="session"
        text="see [seq](knowledge/内核/seq.md)"
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "seq" }));
    expect(useKnowledgeStore.getState().focusedId).toBe("seq");
    expect(useKnowledgeStore.getState().expanded.has("seq")).toBe(false);
  });

  it("marks a dangling knowledge citation and does not focus", () => {
    useKnowledgeStore.setState({
      ready: true,
      refsByNode: new Map([
        [
          "broken-marker",
          new Map([
            [
              "path:knowledge/not-a-node.md",
              {
                refKey: "path:knowledge/not-a-node.md",
                kind: "node" as const,
                key: "not-a-node",
                path: "knowledge/not-a-node.md",
                symbol: null,
                lines: null,
                raw: "[not-a-node](knowledge/not-a-node.md)",
                status: "dangling_relation" as const,
                severity: "error" as const,
                targetPath: null,
                args: { path: "knowledge/not-a-node.md", reason: "missing" },
                message: 'Citation "knowledge/not-a-node.md" does not exist.',
              },
            ],
          ]),
        ],
      ]),
    });
    render(
      <KnowledgeMarkdown
        sourceId="broken-marker"
        text="[not-a-node](knowledge/not-a-node.md)"
      />,
    );
    expect(screen.queryByRole("button", { name: "not-a-node" })).toBeNull();
    expect(
      screen.getByText("not-a-node").closest(".knowledge-token")?.classList.contains("is-invalid"),
    ).toBe(true);
    expect(useKnowledgeStore.getState().focusedId).toBeNull();
  });

  it("does not turn a citation inside a code block into a chip", () => {
    render(
      <KnowledgeMarkdown
        sourceId="session"
        text={"```\n[seq](knowledge/内核/seq.md)\n```"}
      />,
    );
    expect(screen.queryByRole("button", { name: "seq" })).toBeNull();
  });

  it("draws a file and symbol citation as interactive capsules", () => {
    const openFile = vi.fn(async () => {});
    const openFileAt = vi.fn(async () => {});
    useEditorStore.setState({ openFile, openFileAt } as never);
    render(
      <KnowledgeMarkdown
        sourceId="session"
        text="see [a.rs](src/a.rs) and [Store.save](src/a.rs#Store.save)"
      />,
    );
    const file = screen.getByRole("button", { name: "a.rs" });
    expect(file.parentElement?.className).toContain("knowledge-token");
    expect(file.parentElement?.className).toContain("is-file");
    expect(file.parentElement?.className).not.toContain("is-symbol");
    fireEvent.click(file);
    expect(openFile).toHaveBeenCalledWith("src/a.rs");
    const symbol = screen.getByRole("button", { name: "a.rs : save" });
    expect(symbol.parentElement?.className).toContain("is-symbol");
    fireEvent.click(symbol);
    expect(openFile).toHaveBeenCalledTimes(2);
  });

  it("does not treat a bare double-bracket or @text as a citation", () => {
    render(<KnowledgeMarkdown sourceId="session" text="see [[seq]] and @seq" />);
    expect(screen.queryByRole("button", { name: "seq" })).toBeNull();
  });
});

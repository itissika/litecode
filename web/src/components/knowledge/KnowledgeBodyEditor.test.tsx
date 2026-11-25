import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { knowledgeFixture, knowledgeFolderFixture } from "../../test/fixtures/knowledge";
import { useEditorStore } from "../../stores/editorStore";
import { knowledgeSnapshot, useKnowledgeStore } from "../../stores/knowledgeStore";
import { KnowledgeBodyEditor } from "./KnowledgeBodyEditor";

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

describe("KnowledgeBodyEditor mention chip", () => {
  it("jumps to the cited card and keeps the standard Markdown link", async () => {
    const onChange = vi.fn();
    const source = "见 [seq](knowledge/内核/seq.md)";
    render(
      <KnowledgeBodyEditor
        label="正文"
        sourceId="session"
        value={source}
        candidates={[
          {
            key: "seq",
            path: "knowledge/内核/seq.md",
            diskPath: ".litecode/knowledge/内核/seq.md",
          },
        ]}
        rows={3}
        onChange={onChange}
      />,
    );
    const chip = await screen.findByRole("button", { name: "seq" });
    fireEvent.click(chip);
    expect(useKnowledgeStore.getState().focusedId).toBe("seq");
    expect(onChange).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "seq" })).toBeTruthy();
  });

  it("does not jump when the backend marks a knowledge path dangling", async () => {
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
      <KnowledgeBodyEditor
        label="正文"
        sourceId="broken-marker"
        value="[not-a-node](knowledge/not-a-node.md)"
        candidates={[]}
        rows={3}
        onChange={() => undefined}
      />,
    );
    expect(await screen.findByText("not-a-node")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "not-a-node" })).toBeNull();
    expect(useKnowledgeStore.getState().focusedId).toBeNull();
  });

  it("opens a present file and leaves a missing path red", async () => {
    const openFile = vi.fn(async () => {});
    const original = useEditorStore.getState().openFile;
    useEditorStore.setState({ openFile });
    const { unmount } = render(
      <KnowledgeBodyEditor
        label="正文"
        sourceId="session"
        value="[a.rs](src/a.rs)"
        candidates={[]}
        rows={3}
        onChange={() => undefined}
      />,
    );
    fireEvent.click(await screen.findByRole("button", { name: "a.rs" }));
    expect(openFile).toHaveBeenCalledWith("src/a.rs");
    unmount();

    useKnowledgeStore.setState({
      ready: true,
      refsByNode: new Map([
        [
          "session",
          new Map([
            [
              "path:src/a.rs",
              {
                refKey: "path:src/a.rs",
                kind: "file" as const,
                key: null,
                path: "src/a.rs",
                symbol: null,
                lines: null,
                raw: "[a.rs](src/a.rs)",
                status: "missing_file" as const,
                severity: "error" as const,
                targetPath: "src/a.rs",
                args: { path: "src/a.rs" },
                message: 'File "src/a.rs" does not exist.',
              },
            ],
          ]),
        ],
      ]),
    });
    render(
      <KnowledgeBodyEditor
        label="正文"
        sourceId="session"
        value="[a.rs](src/a.rs)"
        candidates={[]}
        rows={3}
        onChange={() => undefined}
      />,
    );
    const label = await screen.findByText("a.rs");
    expect(label.closest(".knowledge-token")?.classList.contains("is-missing")).toBe(true);
    expect(screen.queryByRole("button", { name: "a.rs" })).toBeNull();
    useEditorStore.setState({ openFile: original });
  });
});

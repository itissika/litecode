import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  knowledgeFixture,
  knowledgeFolderFixture,
} from "../../test/fixtures/knowledge";
import {
  knowledgeSnapshot,
  useKnowledgeStore,
} from "../../stores/knowledgeStore";
import { KnowledgeDocument } from "./KnowledgeDocument";

const saveNode = vi.fn(async () => true);

function seed(overrides: Record<string, unknown> = {}) {
  useKnowledgeStore.setState({
    ...knowledgeSnapshot(knowledgeFixture, knowledgeFolderFixture),
    expanded: new Set(),
    graphExpanded: new Set(),
    focusedId: "session",
    focusNonce: 0,
    flashId: null,
    flashNonce: 0,
    loading: false,
    error: null,
    saveNode,
    ...overrides,
  });
}

beforeEach(() => {
  saveNode.mockClear();
  seed();
});

afterEach(() => {
  cleanup();
});

/** The panel keys the document by node, so a chip jump remounts it. */
function KeyedDocument() {
  const focusedId = useKnowledgeStore((s) => s.focusedId);
  return <KnowledgeDocument key={focusedId ?? "empty"} />;
}

describe("KnowledgeDocument", () => {
  it("reads the focused node and follows a body chip in place", () => {
    render(<KeyedDocument />);
    expect(screen.getByRole("heading", { name: "session" })).toBeTruthy();
    // The document header is only the title and its switch: no path or filename.
    expect(screen.queryByText("内核/session.md")).toBeNull();
    const body = document.querySelector(".knowledge-document-body");
    expect(body).not.toBeNull();
    expect(within(body as HTMLElement).getByText(/单调递增/)).toBeTruthy();

    fireEvent.click(within(body as HTMLElement).getByRole("button", { name: "seq" }));

    expect(useKnowledgeStore.getState().focusedId).toBe("seq");
    expect(screen.getByRole("heading", { name: "seq" })).toBeTruthy();
    const nextBody = document.querySelector(".knowledge-document-body");
    expect(within(nextBody as HTMLElement).getByText(/会话日志的序号/)).toBeTruthy();
  });

  it("locates the node in the side list, expanding its ancestors", () => {
    seed({ focusedId: "compact", expandedFolders: new Set() });
    render(<KeyedDocument />);

    const state = useKnowledgeStore.getState();
    expect(state.expandedFolders.has("内核")).toBe(true);
    expect(state.expandedFolders.has("内核/上下文")).toBe(true);
    expect(state.flashId).toBe("compact");
  });

  it("edits the summary directly and toggles the enabled state", async () => {
    render(<KeyedDocument />);
    const summary = screen.getByRole("textbox", { name: "摘要" });
    fireEvent.change(summary, { target: { value: "新的摘要" } });
    fireEvent.blur(summary);
    await waitFor(() => {
      expect(saveNode).toHaveBeenCalledWith("session", { summary: "新的摘要" });
    });

    fireEvent.click(screen.getByRole("switch", { name: "禁用" }));
    await waitFor(() => {
      expect(saveNode).toHaveBeenCalledWith("session", { status: "disabled" });
    });
  });

  it("shows structure information and switches between reading and editing", async () => {
    render(<KeyedDocument />);
    expect(screen.getByText("关联节点")).toBeTruthy();
    expect(screen.getByText("引用", { exact: true })).toBeTruthy();
    expect(screen.getByText("被引用", { exact: true })).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "编辑正文" }));
    expect(document.querySelector(".ProseMirror")).not.toBeNull();
    expect(document.querySelector(".agent-markdown")).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "预览正文" }));
    await waitFor(() => {
      expect(document.querySelector(".agent-markdown")).not.toBeNull();
    });
    expect(document.querySelector(".ProseMirror")).toBeNull();
  });
});

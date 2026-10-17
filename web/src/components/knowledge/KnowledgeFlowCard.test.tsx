import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { ReactFlowProvider, type Node, type NodeProps } from "@xyflow/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { knowledgeFixture, knowledgeFolderFixture } from "../../test/fixtures/knowledge";
import { knowledgeSnapshot, useKnowledgeStore } from "../../stores/knowledgeStore";
import { KnowledgeFlowCard } from "./KnowledgeFlowCard";

beforeEach(() => {
  useKnowledgeStore.setState({
    ...knowledgeSnapshot(knowledgeFixture, knowledgeFolderFixture),
    expanded: new Set(),
    graphExpanded: new Set(["session"]),
    focusedId: null,
    focusNonce: 0,
    flashId: null,
    flashNonce: 0,
    loading: false,
    error: null,
    saveNode: async () => true,
  });
});

afterEach(() => {
  cleanup();
});

function renderCard() {
  const props = {
    id: "session",
    data: { nodeId: "session" },
    type: "knowledge",
    selected: false,
    dragging: false,
  } as NodeProps<Node<{ nodeId: string }, "knowledge">>;
  return render(
    <ReactFlowProvider>
      <KnowledgeFlowCard {...props} />
    </ReactFlowProvider>,
  );
}

describe("KnowledgeFlowCard body", () => {
  it("renders the body with AgentMarkdown and switches to the editor", () => {
    const { container } = renderCard();
    expect(container.querySelector(".agent-markdown")).not.toBeNull();
    expect(container.querySelector(".ProseMirror")).toBeNull();
    expect(container.textContent).toContain("单调递增");

    fireEvent.click(screen.getByRole("button", { name: "编辑正文" }));
    expect(container.querySelector(".ProseMirror")).not.toBeNull();
    expect(container.querySelector(".agent-markdown")).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "预览正文" }));
    expect(container.querySelector(".agent-markdown")).not.toBeNull();
    expect(container.querySelector(".ProseMirror")).toBeNull();
  });
});

import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  knowledgeFixture,
  knowledgeFolderFixture,
} from "../../test/fixtures/knowledge";
import {
  knowledgeSnapshot,
  useKnowledgeStore,
} from "../../stores/knowledgeStore";
import { KnowledgeGraphPanel } from "./KnowledgeGraphPanel";

vi.mock("../../components/knowledge/KnowledgeGraph", () => ({
  KnowledgeGraph: () => <div data-testid="knowledge-canvas" />,
}));

const updateParameters = vi.fn();
const api = {
  id: "knowledge-graph",
  isVisible: true,
  onDidVisibilityChange: () => ({ dispose: () => {} }),
  updateParameters,
} as never;

beforeEach(() => {
  updateParameters.mockClear();
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
    notePanelVisible: async () => {},
  });
});

afterEach(() => {
  cleanup();
});

describe("KnowledgeGraphPanel", () => {
  it("opens as a document and switches surfaces from the bottom capsule", () => {
    render(
      <KnowledgeGraphPanel api={api} containerApi={{} as never} params={{}} />,
    );

    expect(document.querySelector(".knowledge-document")).not.toBeNull();
    expect(screen.queryByTestId("knowledge-canvas")).toBeNull();
    expect(
      screen.getByRole("button", { name: "Document" }).getAttribute("aria-pressed"),
    ).toBe("true");

    fireEvent.click(screen.getByRole("button", { name: "Canvas" }));
    expect(screen.getByTestId("knowledge-canvas")).toBeTruthy();
    expect(document.querySelector(".knowledge-document")).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Document" }));
    expect(document.querySelector(".knowledge-document")).not.toBeNull();
    expect(screen.queryByTestId("knowledge-canvas")).toBeNull();
  });

  it("opens the document stored in the layout params after a restart", () => {
    useKnowledgeStore.setState({ focusedId: null });
    render(
      <KnowledgeGraphPanel
        api={api}
        containerApi={{} as never}
        params={{ nodeId: "seq" }}
      />,
    );

    expect(useKnowledgeStore.getState().focusedId).toBe("seq");
    expect(screen.getByRole("heading", { name: "seq" })).toBeTruthy();
  });

  it("keeps the first-node fallback when the stored document is gone", () => {
    useKnowledgeStore.setState({ focusedId: null });
    const first = useKnowledgeStore.getState().nodes[0].id;
    render(
      <KnowledgeGraphPanel
        api={api}
        containerApi={{} as never}
        params={{ nodeId: "deleted-node" }}
      />,
    );

    expect(useKnowledgeStore.getState().focusedId).toBe(first);
  });

  it("stores each document read in the panel params", () => {
    render(
      <KnowledgeGraphPanel api={api} containerApi={{} as never} params={{}} />,
    );
    expect(updateParameters).toHaveBeenCalledWith({ nodeId: "session" });

    act(() => useKnowledgeStore.getState().focusCanvas("compact"));

    expect(updateParameters).toHaveBeenLastCalledWith({ nodeId: "compact" });
  });
});

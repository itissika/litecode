import { cleanup, fireEvent, render, screen } from "@testing-library/react";
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

const api = {
  id: "knowledge-graph",
  isVisible: true,
  onDidVisibilityChange: () => ({ dispose: () => {} }),
} as never;

beforeEach(() => {
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
});

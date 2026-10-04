import { afterEach, describe, expect, it, vi } from "vitest";

import { setDockviewApi } from "../../stores/connectionStore";
import { useKnowledgeStore } from "../../stores/knowledgeStore";
import { openKnowledgeGraphPanel, revealKnowledgeNode } from "./panel";

afterEach(() => {
  setDockviewApi(null);
});

describe("openKnowledgeGraphPanel", () => {
  it("activates a hidden graph panel instead of adding another", () => {
    const setActive = vi.fn();
    const addPanel = vi.fn();
    setDockviewApi({
      getPanel: vi.fn(() => ({ api: { setActive, isVisible: false } })),
      addPanel,
      groups: [],
      addGroup: vi.fn(),
    } as never);
    openKnowledgeGraphPanel();
    expect(setActive).toHaveBeenCalled();
    expect(addPanel).not.toHaveBeenCalled();
  });

  it("leaves an already visible graph panel where it is", () => {
    const setActive = vi.fn();
    const addPanel = vi.fn();
    setDockviewApi({
      getPanel: vi.fn(() => ({ api: { setActive, isVisible: true } })),
      addPanel,
      groups: [],
      addGroup: vi.fn(),
    } as never);
    openKnowledgeGraphPanel();
    expect(setActive).not.toHaveBeenCalled();
    expect(addPanel).not.toHaveBeenCalled();
  });

  it("splits a new group beside the current grid when none is open", () => {
    const addPanel = vi.fn();
    const addGroup = vi.fn();
    setDockviewApi({
      getPanel: vi.fn(() => undefined),
      addPanel,
      addGroup,
      activeGroup: { api: { id: "edge", location: { type: "edge" } } },
      groups: [
        { api: { location: { type: "edge" }, id: "edge" } },
        { api: { location: { type: "grid" }, id: "g1" } },
      ],
    } as never);
    openKnowledgeGraphPanel();
    expect(addGroup).not.toHaveBeenCalled();
    expect(addPanel).toHaveBeenCalledWith(
      expect.objectContaining({
        id: "knowledge-graph",
        component: "knowledgeGraph",
        title: "Knowledge Graph",
        tabComponent: "knowledgeGraph",
        position: { referenceGroup: "g1", direction: "right" },
      }),
    );
  });

  it("starts a grid group when the center is empty", () => {
    const addPanel = vi.fn();
    const addGroup = vi.fn(() => ({ id: "fresh" }));
    setDockviewApi({
      getPanel: vi.fn(() => undefined),
      addPanel,
      addGroup,
      groups: [],
    } as never);
    openKnowledgeGraphPanel();
    expect(addGroup).toHaveBeenCalled();
    expect(addPanel).toHaveBeenCalledWith(
      expect.objectContaining({
        position: { referenceGroup: "fresh" },
      }),
    );
  });
});

describe("revealKnowledgeNode", () => {
  it("focuses the node and opens the graph", () => {
    useKnowledgeStore.setState({
      byId: new Map([["seq", { id: "seq" }]]),
    } as never);
    const addPanel = vi.fn();
    setDockviewApi({
      getPanel: vi.fn(() => undefined),
      addPanel,
      addGroup: vi.fn(() => ({ id: "fresh" })),
      groups: [],
    } as never);
    revealKnowledgeNode("seq");
    expect(useKnowledgeStore.getState().focusedId).toBe("seq");
    expect(addPanel).toHaveBeenCalled();
  });
});

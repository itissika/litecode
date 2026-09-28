import { afterEach, describe, expect, it, vi } from "vitest";

import { setDockviewApi } from "../../stores/connectionStore";
import { openKnowledgeGraphPanel } from "./panel";

afterEach(() => {
  setDockviewApi(null);
});

describe("openKnowledgeGraphPanel", () => {
  it("activates the existing graph panel", () => {
    const setActive = vi.fn();
    const addPanel = vi.fn();
    setDockviewApi({
      getPanel: vi.fn(() => ({ api: { setActive } })),
      addPanel,
      groups: [],
      addGroup: vi.fn(),
    } as never);
    openKnowledgeGraphPanel();
    expect(setActive).toHaveBeenCalled();
    expect(addPanel).not.toHaveBeenCalled();
  });

  it("adds one graph panel into the grid when none is open", () => {
    const addPanel = vi.fn();
    setDockviewApi({
      getPanel: vi.fn(() => undefined),
      addPanel,
      addGroup: vi.fn(),
      groups: [{ api: { location: { type: "grid" }, id: "g1" } }],
    } as never);
    openKnowledgeGraphPanel();
    expect(addPanel).toHaveBeenCalledWith(
      expect.objectContaining({
        id: "knowledge-graph",
        component: "knowledgeGraph",
        title: "Knowledge Graph",
        tabComponent: "knowledgeGraph",
        position: { referenceGroup: "g1" },
      }),
    );
  });
});

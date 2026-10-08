import { afterEach, describe, expect, it, vi } from "vitest";

import { openPanel, popoutPanel, movePanelToMain } from "./commands";
import { bindDockview } from "./host";
import {
  ensureMainCenterGroup,
  noteActiveGroup,
  placementFor,
  resetPlacementForTests,
} from "./placement";
import type { GroupLike } from "./readGroup";

afterEach(() => {
  bindDockview(null);
  resetPlacementForTests();
});

function group(
  id: string,
  type: "grid" | "popout" | "edge",
  options?: {
    visible?: boolean;
    width?: number;
    height?: number;
    components?: string[];
    setVisible?: (visible: boolean) => void;
  },
): GroupLike {
  return {
    api: {
      id,
      location: { type },
      isVisible: options?.visible ?? true,
      width: options?.width,
      height: options?.height,
      setVisible: options?.setVisible,
    },
    panels: (options?.components ?? []).map((component) => ({
      id: `${component}-panel`,
      api: { component },
    })),
  };
}

describe("ensureMainCenterGroup", () => {
  it("shows a popout anchor again before inserting a group", () => {
    const anchor = group("anchor", "grid", { visible: false, width: 0, height: 0 });
    anchor.api.setVisible = (visible) => {
      anchor.api.isVisible = visible;
      if (visible) {
        anchor.api.width = 240;
        anchor.api.height = 180;
      }
    };
    const addGroup = vi.fn();
    const api = { groups: [anchor], addGroup };
    expect(ensureMainCenterGroup(api as never)).toBe(anchor);
    expect(addGroup).not.toHaveBeenCalled();
  });

  it("inserts a group when the anchor stays at zero size", () => {
    const anchor = group("anchor", "grid", {
      visible: false,
      width: 0,
      height: 0,
      setVisible: () => {},
    });
    const addGroup = vi.fn(() => ({ id: "fresh" }));
    const api = { groups: [anchor], addGroup };
    const created = ensureMainCenterGroup(api as never);
    expect(addGroup).toHaveBeenCalledWith({ referenceGroup: "anchor", direction: "right" });
    expect(created && "api" in created && created.api.id).toBe("fresh");
  });
});

describe("placementFor", () => {
  it("puts an editor to the right of an agent group", () => {
    const agent = group("agent", "grid", { components: ["agent"] });
    const api = { groups: [agent], activeGroup: agent, addGroup: vi.fn() };
    expect(placementFor("editor", api as never)?.position).toEqual({
      referenceGroup: "agent",
      direction: "right",
    });
  });

  it("reuses a document group instead of the agent group", () => {
    const agent = group("agent", "grid", { components: ["agent"] });
    const editors = group("editors", "grid", { components: ["editor"] });
    const api = { groups: [agent, editors], activeGroup: agent, addGroup: vi.fn() };
    expect(placementFor("browser", api as never)?.position).toEqual({
      referenceGroup: "editors",
    });
  });

  it("joins the other main group for a beside panel, using the recent one", () => {
    const src = group("src", "grid", { components: ["agent"] });
    const fresh = group("fresh", "grid", { components: ["editor"] });
    const stale = group("stale", "grid", { components: ["editor"] });
    const api = { groups: [src, fresh, stale], activeGroup: src, addGroup: vi.fn() };
    noteActiveGroup(fresh);
    noteActiveGroup(src);
    expect(placementFor("knowledgeGraph", api as never)?.position).toEqual({
      referenceGroup: "fresh",
    });
  });

  it("keeps a new panel off a foreground popout", () => {
    const pop = group("pop", "popout", { components: ["editor"] });
    const addGroup = vi.fn(() => ({ id: "main" }));
    const api = { groups: [pop], activeGroup: pop, addGroup };
    expect(placementFor("editor", api as never)?.position).toEqual({
      referenceGroup: "main",
    });
  });
});

describe("openPanel", () => {
  it("activates a panel that is already on the main center", () => {
    const setActive = vi.fn();
    const addPanel = vi.fn();
    const host = group("main", "grid", { components: ["editor"] });
    openPanel(
      { id: "editor-panel", component: "editor", title: "a.ts" },
      {
        groups: [host],
        activeGroup: host,
        addPanel,
        addGroup: vi.fn(),
        getPanel: () => ({
          api: {
            component: "editor",
            isVisible: true,
            setActive,
            group: host,
          },
        }),
      } as never,
    );
    expect(setActive).toHaveBeenCalled();
    expect(addPanel).not.toHaveBeenCalled();
  });

  it("moves a panel off an anchor before activating it", () => {
    const setActive = vi.fn();
    const moveTo = vi.fn();
    const addPanel = vi.fn();
    const anchor = group("anchor", "grid", { visible: false, width: 0, height: 0 });
    const main = group("main", "grid");
    openPanel(
      { id: "file", component: "editor", title: "a.ts" },
      {
        groups: [anchor, main],
        addPanel,
        addGroup: vi.fn(),
        getPanel: () => ({
          api: {
            component: "editor",
            setActive,
            moveTo,
            group: anchor,
          },
        }),
      } as never,
    );
    expect(moveTo).toHaveBeenCalledWith({ group: main });
    expect(setActive).toHaveBeenCalled();
    expect(addPanel).not.toHaveBeenCalled();
  });

  it("adds a missing panel on the main center while a popout is in front", () => {
    const addPanel = vi.fn();
    const pop = group("pop", "popout", { components: ["editor"] });
    openPanel(
      { id: "agent-1", component: "agent", title: "NEW" },
      {
        groups: [pop],
        activeGroup: pop,
        addPanel,
        addGroup: vi.fn(() => ({ id: "main" })),
        getPanel: () => undefined,
      } as never,
    );
    expect(addPanel).toHaveBeenCalledWith(
      expect.objectContaining({
        id: "agent-1",
        position: { referenceGroup: "main" },
      }),
    );
  });
});

describe("popout and return", () => {
  it("refuses to pop an edge panel", () => {
    const addPopoutGroup = vi.fn();
    bindDockview({
      addPopoutGroup,
      getPanel: () => ({ api: { component: "terminal", location: { type: "edge" } } }),
    } as never);
    popoutPanel("workspace-terminal");
    expect(addPopoutGroup).not.toHaveBeenCalled();
  });

  it("returns an editor to the document group", () => {
    const moveTo = vi.fn();
    const editors = group("editors", "grid", { components: ["editor"] });
    const agent = group("agent", "grid", { components: ["agent"] });
    bindDockview({
      groups: [agent, editors],
      activeGroup: agent,
      addGroup: vi.fn(),
      getPanel: () => ({
        api: { component: "editor", moveTo, setActive: vi.fn() },
      }),
    } as never);
    movePanelToMain("src/a.ts");
    expect(moveTo).toHaveBeenCalledWith({ group: editors });
  });
});

import { afterEach, describe, expect, it, vi } from "vitest";

import { noteUserWindow, resetActivityForTests } from "./activity";
import { movePanelToMain, openPanel, popoutPanel, routePopoutReturnDrop } from "./commands";
import { bindDockview } from "./host";
import {
  ensureMainCenterGroup,
  noteActiveGroup,
  placementFor,
  resetPlacementForTests,
} from "./placement";
import type { GroupLike } from "./readGroup";
import { bindWindowRegistry, resetWindowsForTests } from "./windows";

const DOCK = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee";

afterEach(() => {
  bindDockview(null);
  resetPlacementForTests();
  resetActivityForTests();
  resetWindowsForTests();
});

function fakePopoutWindow(): Window {
  return {
    closed: false,
    document: document.implementation.createHTMLDocument("popout"),
  } as unknown as Window;
}

/** Register one popout group whose window carries the dock id. */
function registerPopout(win: Window, pop: GroupLike): void {
  let add: (popout: { id: string; window: Window; group: GroupLike }) => void = () => {};
  bindWindowRegistry({
    onDidAddPopoutGroup(cb: (popout: { id: string; window: Window; group: GroupLike }) => void) {
      add = cb;
      return { dispose() {} };
    },
    onDidRemovePopoutGroup() {
      return { dispose() {} };
    },
  } as never);
  pop.api.location = { type: "popout", popoutUrl: `/popout.html?dock=${DOCK}` };
  add({ id: "group-1", window: win, group: pop });
}

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

  it("hosts on a revealed anchor even when size still reads zero", () => {
    const anchor = group("anchor", "grid", {
      visible: false,
      width: 0,
      height: 0,
      setVisible: () => {
        anchor.api.isVisible = true;
      },
    });
    const addGroup = vi.fn(() => ({ id: "fresh" }));
    const api = { groups: [anchor], addGroup };
    expect(ensureMainCenterGroup(api as never)).toBe(anchor);
    expect(addGroup).not.toHaveBeenCalled();
  });

  it("reuses a zero-size main-center instead of addGroup", () => {
    const pending = group("pending", "grid", { width: 0, height: 0 });
    const addGroup = vi.fn(() => ({ id: "fresh" }));
    const api = { groups: [pending], addGroup };
    expect(ensureMainCenterGroup(api as never)).toBe(pending);
    expect(addGroup).not.toHaveBeenCalled();
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

  it("hosts a new panel as a tab in the popout the user is working in", () => {
    const pop = group("pop", "popout", { components: ["editor"] });
    const api = { groups: [pop], activeGroup: pop, addGroup: vi.fn() };
    const win = fakePopoutWindow();
    registerPopout(win, pop);
    expect(placementFor("editor", api as never, undefined, DOCK)?.position).toEqual({
      referenceGroup: "pop",
    });
  });

  it("falls back to the main center for a dock id it does not know", () => {
    const pop = group("pop", "popout", { components: ["editor"] });
    const main = group("main", "grid", { components: ["editor"] });
    const api = { groups: [main, pop], activeGroup: pop, addGroup: vi.fn() };
    expect(placementFor("editor", api as never, undefined, "gone")?.position).toEqual({
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

  it("adds a missing panel to the popout the user last used", () => {
    const addPanel = vi.fn();
    const pop = group("pop", "popout", { components: ["editor"] });
    const win = fakePopoutWindow();
    registerPopout(win, pop);
    noteUserWindow(win);
    openPanel(
      { id: "src/b.ts", component: "editor", title: "b.ts" },
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
        id: "src/b.ts",
        position: { referenceGroup: "pop" },
      }),
    );
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

  it("opens a dragged tab at the pointer, not at a minimized main window", () => {
    const addPopoutGroup = vi.fn(() => Promise.resolve(true));
    bindDockview({
      addPopoutGroup,
      getPanel: () => ({
        api: {
          component: "editor",
          location: { type: "popout" },
          getWindow: () => ({
            closed: false,
            screenX: 480,
            screenY: 160,
            innerWidth: 900,
            innerHeight: 700,
          }),
          group: {
            element: {
              getBoundingClientRect: () => ({ left: 0, top: 0, width: 900, height: 700 }),
            },
          },
        },
      }),
    } as never);
    popoutPanel("src/a.ts", { screenX: 1600, screenY: 400 });
    expect(addPopoutGroup).toHaveBeenCalledWith(
      expect.anything(),
      expect.objectContaining({
        position: { left: 1600, top: 400, width: 900, height: 700 },
      }),
    );
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

/**
 * Mimics Dockview moving the last tab out of a popout window: the popout
 * group is removed, and its reference group on the main grid is removed too
 * when it has no panels, before the panel is opened on the destination.
 */
function popoutReturnDock(mainGroups: GroupLike[], refId: string) {
  const pop = group("pop", "popout");
  const panelApi: Record<string, unknown> = { component: "editor", setActive: vi.fn(), group: pop };
  const entry = { id: "src/a.ts", api: panelApi };
  pop.panels = [entry as never];
  let fresh = 0;
  const live = {
    groups: [...mainGroups, pop] as GroupLike[],
    activeGroup: pop as GroupLike | undefined,
    getPanel: (id: string) => (id === entry.id ? entry : undefined),
    getGroup: (id: string) => live.groups.find((item) => item.api.id === id),
    removeGroup: vi.fn((target: GroupLike) => {
      live.groups = live.groups.filter((item) => item !== target);
    }),
    addGroup: vi.fn(() => {
      const created = group(`fresh-${fresh++}`, "grid", { width: 400, height: 300 });
      live.groups = [created, ...live.groups];
      return created;
    }),
  };
  panelApi.moveTo = vi.fn(({ group: dest }: { group: GroupLike }) => {
    const source = panelApi.group as GroupLike;
    source.panels = [];
    live.groups = live.groups.filter((item) => item !== source);
    const ref = live.groups.find((item) => item.api.id === refId);
    if (ref && (ref.panels ?? []).length === 0) live.groups = live.groups.filter((item) => item !== ref);
    dest.panels = [...(dest.panels ?? []), entry as never];
    panelApi.group = dest;
  });
  const shownIn = () =>
    live.groups.find(
      (item) =>
        item.api.location?.type === "grid" &&
        item.api.isVisible !== false &&
        (item.panels ?? []).some((panel) => panel.id === entry.id),
    );
  return { live, panelApi, shownIn };
}

describe("returning a popout's last tab to the main window", () => {
  it("shows the tab when the main center is an empty group", () => {
    const ref = group("ref", "grid", { width: 800, height: 600 });
    const { live, panelApi, shownIn } = popoutReturnDock([ref], "ref");
    bindDockview(live as never);
    movePanelToMain("src/a.ts");
    expect(live.addGroup).toHaveBeenCalledWith({ referenceGroup: "ref", direction: "right" });
    expect(shownIn()?.api.id).toBe("fresh-0");
    expect(live.groups.some((item) => item.api.id === "ref")).toBe(false);
    expect(panelApi.setActive).toHaveBeenCalled();
  });

  it("shows the tab when only the invisible anchor is left", () => {
    const anchor = group("anchor", "grid", { visible: false, width: 0, height: 0 });
    anchor.api.setVisible = (visible) => {
      anchor.api.isVisible = visible;
    };
    const { live, shownIn } = popoutReturnDock([anchor], "anchor");
    bindDockview(live as never);
    movePanelToMain("src/a.ts");
    expect(shownIn()?.api.id).toBe("fresh-0");
    expect(live.groups.some((item) => item.api.id === "anchor")).toBe(false);
  });

  it("shows the tab when the center is watermark-only", () => {
    const { live, shownIn } = popoutReturnDock([], "gone");
    bindDockview(live as never);
    movePanelToMain("src/a.ts");
    expect(shownIn()).toBeDefined();
    expect(live.groups.filter((item) => item.api.location?.type === "grid")).toHaveLength(1);
  });

  it("joins the existing document group when the center has panels", () => {
    const anchor = group("anchor", "grid", { visible: false, width: 0, height: 0 });
    const editors = group("editors", "grid", { width: 800, height: 600, components: ["editor"] });
    const { live, panelApi, shownIn } = popoutReturnDock([anchor, editors], "anchor");
    bindDockview(live as never);
    movePanelToMain("src/a.ts");
    expect(panelApi.moveTo).toHaveBeenCalledWith({ group: editors });
    expect(live.addGroup).not.toHaveBeenCalled();
    expect(shownIn()?.api.id).toBe("editors");
  });

  it("moves straight onto an empty group when other tabs stay in the popout", () => {
    const ref = group("ref", "grid", { width: 800, height: 600 });
    const { live, panelApi } = popoutReturnDock([ref], "ref");
    const pop = live.groups.find((item) => item.api.id === "pop")!;
    pop.panels = [...(pop.panels ?? []), { id: "other", api: { component: "editor" } }];
    bindDockview(live as never);
    movePanelToMain("src/a.ts");
    expect(panelApi.moveTo).toHaveBeenCalledWith({ group: ref });
    expect(live.addGroup).not.toHaveBeenCalled();
  });

  it("takes over a native drop onto an empty main group", () => {
    const ref = group("ref", "grid", { width: 800, height: 600 });
    const { live, shownIn } = popoutReturnDock([ref], "ref");
    const preventDefault = vi.fn();
    const handled = routePopoutReturnDrop(
      { getData: () => ({ panelId: "src/a.ts" }), group: ref, preventDefault },
      live as never,
    );
    expect(handled).toBe(true);
    expect(preventDefault).toHaveBeenCalled();
    expect(shownIn()?.api.id).toBe("fresh-0");
  });

  it("leaves a native drop onto a group with tabs to Dockview", () => {
    const editors = group("editors", "grid", { components: ["editor"] });
    const { live, panelApi } = popoutReturnDock([editors], "gone");
    const preventDefault = vi.fn();
    expect(
      routePopoutReturnDrop(
        { getData: () => ({ panelId: "src/a.ts" }), group: editors, preventDefault },
        live as never,
      ),
    ).toBe(false);
    expect(preventDefault).not.toHaveBeenCalled();
    expect(panelApi.moveTo).not.toHaveBeenCalled();
  });
});

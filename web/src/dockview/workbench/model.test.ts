import { describe, expect, it } from "vitest";

import { groupRole, isUsableCenter, type GroupFacts } from "./model";
import {
  activeCenterSessionIds,
  centerGroups,
  watchActiveCenterSessionIds,
  whereIs,
} from "./queries";
import { bindDockview } from "./host";

function facts(patch: Partial<GroupFacts>): GroupFacts {
  return {
    id: "g",
    locationType: "grid",
    isVisible: true,
    width: undefined,
    height: undefined,
    panels: [],
    ...patch,
  };
}

describe("group roles", () => {
  it("classifies the main center, a popout, an anchor, and an edge", () => {
    expect(groupRole(facts({ locationType: "grid", isVisible: true }))).toBe("main-center");
    expect(groupRole(facts({ locationType: "popout", isVisible: true }))).toBe("popout-center");
    expect(groupRole(facts({ locationType: "grid", isVisible: false }))).toBe("popout-anchor");
    expect(groupRole(facts({ locationType: "edge", isVisible: true }))).toBe("edge");
    expect(groupRole(facts({ locationType: undefined }))).toBeNull();
  });

  it("treats a zero-size center as unusable and an unmeasured one as usable", () => {
    expect(isUsableCenter(facts({ width: 0, height: 0 }))).toBe(false);
    expect(isUsableCenter(facts({}))).toBe(true);
    expect(isUsableCenter(facts({ locationType: "grid", isVisible: false, width: 40 }))).toBe(
      false,
    );
    expect(isUsableCenter(facts({ locationType: "popout", width: 0, height: 12 }))).toBe(true);
  });
});

describe("center group queries", () => {
  it("keeps an empty grid, a live popout, and an anchor apart", () => {
    const main = {
      api: { id: "main", location: { type: "grid" }, isVisible: true, width: 400, height: 300 },
      panels: [],
    };
    const anchor = {
      api: { id: "anchor", location: { type: "grid" }, isVisible: false, width: 0, height: 0 },
      panels: [],
    };
    const popout = {
      api: {
        id: "pop",
        location: { type: "popout", popoutUrl: "/popout.html?dock=aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee" },
        isVisible: true,
        width: 200,
        height: 200,
      },
      panels: [{ id: "agent-1", api: { component: "agent" } }],
    };
    const edge = {
      api: { id: "left", location: { type: "edge" }, isVisible: true },
      panels: [{ id: "filetree", api: { component: "filetree" } }],
    };
    const api = {
      groups: [main, anchor, popout, edge],
      panels: [
        { id: "agent-1", api: { component: "agent", group: popout, location: { type: "popout" } } },
      ],
      getPanel: (id: string) =>
        id === "agent-1"
          ? { id, api: { component: "agent", group: popout, location: { type: "popout" } } }
          : undefined,
      getPopouts: () => [{ id: "pop", group: popout, window: { location: { href: "" } } }],
    };
    bindDockview(api as never);

    expect(centerGroups({ window: "main" }).map((group) => group.id)).toEqual([
      "main",
      "anchor",
    ]);
    expect(centerGroups({ window: "main", usable: true }).map((group) => group.id)).toEqual([
      "main",
    ]);
    expect(centerGroups({ window: "popout", usable: true }).map((group) => group.id)).toEqual([
      "pop",
    ]);
    expect(whereIs("agent-1")).toMatchObject({
      role: "popout-center",
      zone: "center",
      window: { dockId: "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee" },
    });
    bindDockview(null);
  });

  it("reports the session each live center group shows, not its edge or anchor", () => {
    const panel = (sessionId: string) => ({
      id: `agent-${sessionId}`,
      params: { sessionId },
      api: { component: "agent" },
    });
    const main = {
      api: { id: "main", location: { type: "grid" }, isVisible: true },
      activePanel: panel("s1"),
      panels: [panel("s1"), panel("s2")],
    };
    const anchor = {
      api: { id: "anchor", location: { type: "grid" }, isVisible: false },
      activePanel: panel("s3"),
      panels: [panel("s3")],
    };
    const edge = {
      api: { id: "edge", location: { type: "edge" }, isVisible: true },
      activePanel: {
        id: "sessions",
        params: {},
        api: { component: "sessions" },
      },
      panels: [],
    };
    bindDockview({ groups: [main, anchor, edge] } as never);

    expect(activeCenterSessionIds()).toEqual(["s1"]);
    bindDockview(null);
  });

  it("re-reads after the next frame when a restore lands without a layout event", async () => {
    let groups: unknown[] = [];
    const api = {
      get groups() {
        return groups;
      },
      getPanel: () => undefined,
      onDidLayoutChange: () => ({ dispose: () => {} }),
    };
    bindDockview(api as never);
    const seen: string[][] = [];
    const stop = watchActiveCenterSessionIds((ids) => seen.push([...ids]));
    try {
      groups = [
        {
          api: { id: "main", location: { type: "grid" }, isVisible: true },
          activePanel: {
            id: "agent-s1",
            params: { sessionId: "s1" },
            api: { component: "agent" },
          },
          panels: [],
        },
      ];
      await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
      expect(seen.at(-1)).toEqual(["s1"]);
    } finally {
      stop();
      bindDockview(null);
    }
  });
});

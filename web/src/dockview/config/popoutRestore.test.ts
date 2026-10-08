import { describe, expect, it } from "vitest";

import { popoutPageUrl } from "./popoutUrl";
import { preparePopoutRestore } from "./popoutRestore";

const DOCK_A = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee";
const DOCK_B = "bbbbbbbb-bbbb-4ccc-8ddd-eeeeeeeeeeee";

const origin = { x: 40, y: 20 };

function group(id: string) {
  return { id, views: [id], activeView: id };
}

describe("preparePopoutRestore", () => {
  it("leaves a layout with no popouts untouched", () => {
    const layout = { grid: { root: { type: "branch", data: [] } } };
    expect(preparePopoutRestore(layout, origin)).toEqual({ layout, bounds: new Map() });
    const empty = { ...layout, popoutGroups: [] as unknown[] };
    expect(preparePopoutRestore(empty, origin).layout).toBe(empty);
  });

  it("reopens a popout at its saved screen rectangle", () => {
    const layout = {
      grid: { root: { type: "branch", data: [{ type: "leaf", data: group("editors") }] } },
      popoutGroups: [
        {
          data: group("browser-group"),
          url: `http://127.0.0.1:9/popout.html?dock=${DOCK_A}`,
          position: { left: 500, top: 200, width: 800, height: 600 },
        },
      ],
    };
    const { layout: next, bounds } = preparePopoutRestore(layout, origin);
    expect(next.grid).toEqual(layout.grid);
    expect(next.popoutGroups).toEqual([
      {
        data: group("browser-group"),
        url: popoutPageUrl(DOCK_A),
        position: { left: 460, top: 180, width: 800, height: 600 },
      },
    ]);
    expect(bounds.get(DOCK_A)).toEqual({ x: 500, y: 200, width: 800, height: 600 });
    expect(layout.popoutGroups[0]?.position).toEqual({ left: 500, top: 200, width: 800, height: 600 });
  });

  it("gives a popout without a measured size a fixed window, staggered from the opener", () => {
    const { layout, bounds } = preparePopoutRestore(
      {
        popoutGroups: [
          { data: group("one"), url: popoutPageUrl(DOCK_A) },
          { data: group("two"), url: popoutPageUrl(DOCK_B), position: { width: 40, height: 10 } },
        ],
      },
      origin,
    );
    const groups = layout.popoutGroups as unknown as {
      position: { left: number; top: number; width: number; height: number };
    }[];
    expect(groups[0]?.position).toEqual({ left: 48, top: 48, width: 960, height: 640 });
    expect(groups[1]?.position).toEqual({ left: 84, top: 84, width: 960, height: 640 });
    expect(bounds.get(DOCK_A)).toEqual({ x: 88, y: 68, width: 960, height: 640 });
    expect(bounds.get(DOCK_B)?.x).toBe(124);
  });

  it("keeps a popout that sits on a monitor to the left of the opener", () => {
    const { layout, bounds } = preparePopoutRestore(
      {
        popoutGroups: [
          {
            data: group("side"),
            url: popoutPageUrl(DOCK_A),
            position: { left: -1400, top: 80, width: 900, height: 700 },
          },
        ],
      },
      origin,
    );
    const groups = layout.popoutGroups as unknown as {
      position: { left: number; top: number; width: number; height: number };
    }[];
    expect(groups[0]?.position).toEqual({ left: -1440, top: 60, width: 900, height: 700 });
    expect(bounds.get(DOCK_A)).toEqual({ x: -1400, y: 80, width: 900, height: 700 });
  });

  it("drops a popout entry that has no group payload", () => {
    const { layout, bounds } = preparePopoutRestore(
      {
        grid: { root: { type: "leaf", data: group("keep") } },
        popoutGroups: [{ position: { left: 1, top: 2, width: 800, height: 600 } }],
      },
      origin,
    );
    expect(layout.popoutGroups).toEqual([]);
    expect(bounds.size).toBe(0);
    expect(layout.grid.root).toEqual({ type: "leaf", data: group("keep") });
  });
});


import { describe, expect, it } from "vitest";

import { dockIdFromLocation, isCenterDock, isMainGrid } from "./location";

const DOCK = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee";

describe("dock location", () => {
  it("keeps the main grid distinct from a popout in the same center dock", () => {
    expect(isMainGrid("grid")).toBe(true);
    expect(isMainGrid("popout")).toBe(false);
    expect(isMainGrid("edge")).toBe(false);
    expect(isCenterDock("grid")).toBe(true);
    expect(isCenterDock("popout")).toBe(true);
    expect(isCenterDock("edge")).toBe(false);
  });

  it("reads the electron window id from a popout location", () => {
    expect(
      dockIdFromLocation({ type: "popout", popoutUrl: `/popout.html?dock=${DOCK}` }),
    ).toBe(DOCK);
    expect(dockIdFromLocation({ type: "grid" })).toBeNull();
    expect(dockIdFromLocation(undefined)).toBeNull();
  });
});

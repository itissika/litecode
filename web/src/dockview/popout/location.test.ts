import { describe, expect, it } from "vitest";

import { dockIdFromLocation } from "./location";

const DOCK = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee";

describe("dock location", () => {
  it("reads the electron window id from a popout location", () => {
    expect(
      dockIdFromLocation({ type: "popout", popoutUrl: `/popout.html?dock=${DOCK}` }),
    ).toBe(DOCK);
    expect(dockIdFromLocation({ type: "grid" })).toBeNull();
    expect(dockIdFromLocation(undefined)).toBeNull();
  });
});

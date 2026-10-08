import { describe, expect, it } from "vitest";

import {
  MAIN_WINDOW_KEY,
  decideDockTarget,
  decideRelease,
  rejectsDockTarget,
  windowAtPoint,
  type ScreenWindow,
} from "./tabDragPolicy";

const main: ScreenWindow = {
  key: MAIN_WINDOW_KEY,
  box: { screenX: 0, screenY: 0, outerWidth: 1000, outerHeight: 800 },
};
const popout: ScreenWindow = {
  key: "pop-1",
  box: { screenX: 100, screenY: 100, outerWidth: 200, outerHeight: 200 },
};
const other: ScreenWindow = {
  key: "pop-2",
  box: { screenX: 2000, screenY: 0, outerWidth: 400, outerHeight: 300 },
};

describe("decideDockTarget", () => {
  it("lets a center tab move inside the center dock, including a root split", () => {
    expect(decideDockTarget("center", "center")).toBe("allow");
    expect(decideDockTarget("center", "root")).toBe("allow");
    expect(rejectsDockTarget("grid", "grid")).toBe(false);
    expect(rejectsDockTarget("grid", "popout")).toBe(false);
    expect(rejectsDockTarget("popout", "grid")).toBe(false);
    expect(rejectsDockTarget("popout", undefined)).toBe(false);
    expect(rejectsDockTarget("grid", undefined)).toBe(false);
  });

  it("refuses a center tab on an edge rail and an edge tab everywhere else", () => {
    expect(decideDockTarget("center", "edge")).toBe("reject");
    expect(decideDockTarget("edge", "center")).toBe("reject");
    expect(decideDockTarget("edge", "root")).toBe("reject");
    expect(decideDockTarget("edge", "edge")).toBe("allow");
    expect(rejectsDockTarget("grid", "edge")).toBe(true);
    expect(rejectsDockTarget("edge", "grid")).toBe(true);
    expect(rejectsDockTarget("edge", "popout")).toBe(true);
    expect(rejectsDockTarget("popout", "edge")).toBe(true);
    expect(rejectsDockTarget("edge", "edge")).toBe(false);
    expect(rejectsDockTarget("edge", undefined)).toBe(true);
  });

  it("leaves an unknown drag to dockview", () => {
    expect(rejectsDockTarget(undefined, "edge")).toBe(false);
    expect(rejectsDockTarget("floating", "grid")).toBe(false);
  });
});

describe("decideRelease", () => {
  it("opens a new window only when a center tab is outside every window", () => {
    expect(decideRelease("center", null)).toBe("popout");
    expect(decideRelease("center", MAIN_WINDOW_KEY)).toBe("stay");
    expect(decideRelease("center", "pop-1")).toBe("join");
  });

  it("keeps an edge tab where it is", () => {
    expect(decideRelease("edge", null)).toBe("stay");
    expect(decideRelease("edge", "pop-1")).toBe("stay");
    expect(decideRelease(null, null)).toBe("stay");
  });
});

describe("windowAtPoint", () => {
  const windows = [main, popout, other];

  it("picks the smaller window when a popout overlaps the main window", () => {
    expect(windowAtPoint({ screenX: 150, screenY: 150 }, windows)).toBe("pop-1");
    expect(windowAtPoint({ screenX: 10, screenY: 10 }, windows)).toBe(MAIN_WINDOW_KEY);
  });

  it("reports a point on the desktop as outside", () => {
    expect(windowAtPoint({ screenX: 5000, screenY: 10 }, windows)).toBeNull();
    expect(windowAtPoint({ screenX: 2100, screenY: 20 }, windows)).toBe("pop-2");
  });
});

import { describe, expect, it } from "vitest";

import { popoutScreenBox } from "./popoutBox";

const host = {
  screenX: 400,
  screenY: 120,
  innerWidth: 900,
  innerHeight: 700,
};

const rect = { left: 8, top: 24, width: 880, height: 640 };

describe("popoutScreenBox", () => {
  it("opens a dragged tab at the release point, sized like its group", () => {
    expect(popoutScreenBox(host, rect, { screenX: 1400.4, screenY: 300.2 })).toEqual({
      left: 1400,
      top: 300,
      width: 880,
      height: 640,
    });
  });

  it("uses the window that holds the tab when there is no release point", () => {
    expect(popoutScreenBox(host, rect, null)).toEqual({
      left: 408,
      top: 144,
      width: 880,
      height: 640,
    });
  });

  it("does not use a minimized opener when the tab lives in another window", () => {
    const minimizedOpener = { screenX: -32000, screenY: -32000, innerWidth: 1280, innerHeight: 800 };
    expect(popoutScreenBox(host, rect)).not.toEqual(
      popoutScreenBox(minimizedOpener, rect),
    );
    expect(popoutScreenBox(host, rect)?.left).toBe(408);
  });

  it("falls back to the host size when the group has no box yet", () => {
    expect(popoutScreenBox(host, { left: 0, top: 0, width: 0, height: 0 })).toEqual({
      left: 400,
      top: 120,
      width: 900,
      height: 700,
    });
  });
});

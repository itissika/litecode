import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { boundsOnDisplay, popoutBoundsFromFeatures } from "./popout-bounds";

describe("popoutBoundsFromFeatures", () => {
  it("reads the screen rectangle dockview passes to window.open", () => {
    assert.deepEqual(popoutBoundsFromFeatures("top=200,left=500,width=800,height=600"), {
      x: 500,
      y: 200,
      width: 800,
      height: 600,
    });
  });

  it("keeps a window on a monitor to the left of the origin", () => {
    assert.deepEqual(popoutBoundsFromFeatures("top=80.4,left=-1400.2,width=900,height=700"), {
      x: -1400,
      y: 80,
      width: 900,
      height: 700,
    });
  });

  it("ignores a feature string that does not name a full rectangle", () => {
    assert.equal(popoutBoundsFromFeatures(""), null);
    assert.equal(popoutBoundsFromFeatures(undefined), null);
    assert.equal(popoutBoundsFromFeatures("top=10,left=10"), null);
    assert.equal(popoutBoundsFromFeatures("top=nope,left=1,width=2,height=3"), null);
  });
});

describe("boundsOnDisplay", () => {
  it("leaves a window that already meets a display where it is", () => {
    const areas = [{ x: 0, y: 0, width: 1920, height: 1080 }];
    const bounds = { x: 400, y: 200, width: 800, height: 600 };
    assert.deepEqual(boundsOnDisplay(bounds, areas, { x: 10, y: 10 }), bounds);
  });

  it("keeps a window on a monitor whose origin is left of zero", () => {
    const areas = [{ x: -1920, y: 0, width: 1920, height: 1080 }];
    const bounds = { x: -1400, y: 80, width: 900, height: 700 };
    assert.deepEqual(boundsOnDisplay(bounds, areas, { x: -100, y: 40 }), bounds);
  });

  it("pulls a window opened at a minimized opener back onto the display under the pointer", () => {
    const areas = [{ x: 0, y: 0, width: 1920, height: 1080 }];
    assert.deepEqual(
      boundsOnDisplay({ x: -32000, y: -32000, width: 800, height: 600 }, areas, { x: 500, y: 240 }),
      { x: 500, y: 240, width: 800, height: 600 },
    );
  });
});

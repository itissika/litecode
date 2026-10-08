import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { popoutBoundsFromFeatures } from "./popout-bounds";

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

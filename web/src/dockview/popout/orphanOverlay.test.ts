import { describe, expect, it } from "vitest";

import { releaseOrphanRenderOverlays } from "./orphanOverlay";

describe("releaseOrphanRenderOverlays", () => {
  it("removes empty overlay shells and leaves overlays that still hold content", () => {
    const doc = document.implementation.createHTMLDocument("Litecode");
    doc.body.innerHTML = `
      <div class="dv-render-overlay" id="stale"></div>
      <div class="dv-render-overlay" id="live"><div>panel</div></div>
    `;
    releaseOrphanRenderOverlays([doc]);
    expect(doc.getElementById("stale")).toBeNull();
    expect(doc.getElementById("live")).not.toBeNull();
  });
});

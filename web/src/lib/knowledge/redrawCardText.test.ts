import { describe, expect, it, vi } from "vitest";

import {
  intersectsViewport,
  nextTextRaster,
  redrawVisibleCardText,
  shouldSharpenZoom,
} from "./redrawCardText";

describe("shouldSharpenZoom", () => {
  it("refreshes only after the zoom changed", () => {
    expect(shouldSharpenZoom(null, 1)).toBe(false);
    expect(shouldSharpenZoom(1, 1)).toBe(false);
    expect(shouldSharpenZoom(0.2, 1)).toBe(true);
  });
});

describe("nextTextRaster", () => {
  it("alternates so the text layer transform changes every settle", () => {
    expect(nextTextRaster(null)).toBe("a");
    expect(nextTextRaster("a")).toBe("b");
    expect(nextTextRaster("b")).toBe("a");
  });
});

describe("intersectsViewport", () => {
  const view = new DOMRect(0, 0, 800, 600);

  it("keeps a card that overlaps the pane", () => {
    expect(intersectsViewport(new DOMRect(760, 10, 80, 40), view)).toBe(true);
  });

  it("drops a card fully outside the pane", () => {
    expect(intersectsViewport(new DOMRect(900, 10, 80, 40), view)).toBe(false);
  });
});

describe("redrawVisibleCardText", () => {
  it("marks on-screen cards and leaves the rest", () => {
    const root = document.createElement("div");
    vi.spyOn(root, "getBoundingClientRect").mockReturnValue(new DOMRect(0, 0, 800, 600));
    const visible = document.createElement("div");
    visible.className = "knowledge-flow-card";
    const hidden = document.createElement("div");
    hidden.className = "knowledge-flow-card";
    vi.spyOn(visible, "getBoundingClientRect").mockReturnValue(new DOMRect(10, 10, 120, 80));
    vi.spyOn(hidden, "getBoundingClientRect").mockReturnValue(new DOMRect(2000, 10, 120, 80));
    root.append(visible, hidden);

    redrawVisibleCardText(root);
    expect(visible.getAttribute("data-text-raster")).toBe("a");
    expect(hidden.hasAttribute("data-text-raster")).toBe(false);

    redrawVisibleCardText(root);
    expect(visible.getAttribute("data-text-raster")).toBe("b");
  });
});

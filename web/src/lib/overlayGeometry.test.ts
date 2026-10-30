import { describe, expect, it, vi } from "vitest";

import { elementRect, isElementVisible, rectsOverlap } from "./overlayGeometry";

function withRect(element: HTMLElement, rect: Partial<DOMRect>): void {
  vi.spyOn(element, "getBoundingClientRect").mockReturnValue({
    x: 0,
    y: 0,
    width: 0,
    height: 0,
    top: 0,
    left: 0,
    right: 0,
    bottom: 0,
    toJSON: () => ({}),
    ...rect,
  } as DOMRect);
}

function withVisibility(element: HTMLElement, visible: boolean): void {
  (element as HTMLElement & { checkVisibility: () => boolean }).checkVisibility = () => visible;
}

describe("rectsOverlap", () => {
  it("detects intersecting rectangles", () => {
    expect(
      rectsOverlap(
        { left: 0, top: 0, right: 10, bottom: 10 },
        { left: 5, top: 5, right: 15, bottom: 15 },
      ),
    ).toBe(true);
  });

  it("treats touching edges as no overlap", () => {
    expect(
      rectsOverlap(
        { left: 0, top: 0, right: 10, bottom: 10 },
        { left: 10, top: 0, right: 20, bottom: 10 },
      ),
    ).toBe(false);
  });

  it("detects a fully contained rectangle", () => {
    expect(
      rectsOverlap(
        { left: 0, top: 0, right: 100, bottom: 100 },
        { left: 10, top: 10, right: 20, bottom: 20 },
      ),
    ).toBe(true);
  });
});

describe("isElementVisible", () => {
  it("uses checkVisibility when the browser provides it", () => {
    const element = document.createElement("div");
    document.body.appendChild(element);
    withVisibility(element, false);
    expect(isElementVisible(element)).toBe(false);
    withVisibility(element, true);
    expect(isElementVisible(element)).toBe(true);
    element.remove();
  });

  it("rejects detached elements", () => {
    const element = document.createElement("div");
    expect(isElementVisible(element)).toBe(false);
  });
});

describe("elementRect", () => {
  it("returns a viewport rect for a visible element", () => {
    const element = document.createElement("div");
    document.body.appendChild(element);
    withVisibility(element, true);
    withRect(element, { left: 12, top: 20, right: 52, bottom: 60, width: 40, height: 40 });
    expect(elementRect(element)).toEqual({ left: 12, top: 20, right: 52, bottom: 60 });
    element.remove();
  });

  it("rejects zero-sized and invisible elements", () => {
    const element = document.createElement("div");
    document.body.appendChild(element);
    withVisibility(element, true);
    withRect(element, { left: 12, top: 20, right: 12, bottom: 20, width: 0, height: 0 });
    expect(elementRect(element)).toBeNull();
    withVisibility(element, false);
    withRect(element, { left: 12, top: 20, right: 52, bottom: 60, width: 40, height: 40 });
    expect(elementRect(element)).toBeNull();
    element.remove();
  });
});

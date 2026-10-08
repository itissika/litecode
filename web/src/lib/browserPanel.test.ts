import { describe, expect, it } from "vitest";

import {
  displayBrowserUrl,
  normalizeBrowserUrl,
  showBrowserAddButton,
  browserBoundsPlace,
} from "./browserPanel";

describe("normalizeBrowserUrl", () => {
  it("adds https to a bare host and keeps http", () => {
    expect(normalizeBrowserUrl("example.com/docs")).toBe("https://example.com/docs");
    expect(normalizeBrowserUrl("http://127.0.0.1:3000")).toBe("http://127.0.0.1:3000/");
  });

  it("rejects other schemes and empty input", () => {
    expect(normalizeBrowserUrl("file:///c:/windows")).toBeNull();
    expect(normalizeBrowserUrl("javascript:alert(1)")).toBeNull();
    expect(normalizeBrowserUrl("   ")).toBeNull();
  });
});

describe("displayBrowserUrl", () => {
  it("hides a blank page", () => {
    expect(displayBrowserUrl("")).toBe("");
    expect(displayBrowserUrl("about:blank")).toBe("");
    expect(displayBrowserUrl("https://example.com/")).toBe("https://example.com/");
  });
});

describe("showBrowserAddButton", () => {
  it("shows the add button only on the main center", () => {
    expect(showBrowserAddButton("main-center", true)).toBe(true);
    expect(showBrowserAddButton("edge", true)).toBe(false);
    expect(showBrowserAddButton("main-center", false)).toBe(false);
    expect(showBrowserAddButton(null, true)).toBe(false);
    expect(showBrowserAddButton("popout-center", true)).toBe(false);
    expect(showBrowserAddButton("popout-anchor", true)).toBe(false);
  });
});

describe("browserBoundsPlace", () => {
  it("sends a rectangle only when the element and the native view share a window", () => {
    expect(browserBoundsPlace(true, "main")).toBe("main");
    expect(browserBoundsPlace(false, "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee")).toBe(
      "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee",
    );
    expect(browserBoundsPlace(false, "main")).toBeNull();
    expect(browserBoundsPlace(true, "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee")).toBeNull();
  });
});

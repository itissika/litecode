import { describe, expect, it } from "vitest";

import { viewOf } from "./domView";

describe("viewOf", () => {
  it("uses the window that owns the node", () => {
    const iframe = document.createElement("iframe");
    document.body.appendChild(iframe);
    const childDoc = iframe.contentDocument;
    const childView = iframe.contentWindow;
    expect(childDoc).toBeTruthy();
    expect(childView).toBeTruthy();
    const node = childDoc!.createElement("div");
    childDoc!.body.appendChild(node);
    expect(viewOf(node)).toBe(childView);
    iframe.remove();
  });

  it("falls back to the opener when the node has no window", () => {
    expect(viewOf(null)).toBe(window);
  });
});

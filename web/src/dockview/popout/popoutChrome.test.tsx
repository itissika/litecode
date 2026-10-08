import { afterEach, describe, expect, it, vi } from "vitest";

import { preparePopoutDocument, watchPopoutLayout } from "./popoutChrome";

describe("preparePopoutDocument", () => {
  it("copies the main dock theme onto the popout document", () => {
    const doc = document.implementation.createHTMLDocument("Litecode");
    doc.body.innerHTML = '<div id="dv-popout-window"></div>';
    preparePopoutDocument(doc, "light");
    expect(doc.documentElement.getAttribute("data-dv-theme")).toBe("light");
    expect(doc.documentElement.classList.contains("litecode-dv-base")).toBe(true);
    expect(doc.documentElement.classList.contains("litecode-popout")).toBe(true);
    const shell = doc.getElementById("dv-popout-window") as HTMLElement;
    expect(shell.style.top).toBe("");
    expect(shell.style.height).toBe("");
  });
});

describe("watchPopoutLayout", () => {
  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  it("re-dispatches resize when the content box changes", () => {
    vi.useFakeTimers();
    const spy = vi.spyOn(window, "dispatchEvent");
    const previous = window.innerWidth;
    const stop = watchPopoutLayout(window);
    const resizeCalls = () =>
      spy.mock.calls.filter(([event]) => event instanceof Event && event.type === "resize")
        .length;

    vi.advanceTimersByTime(100);
    expect(resizeCalls()).toBe(0);

    Object.defineProperty(window, "innerWidth", {
      configurable: true,
      value: previous + 40,
    });
    vi.advanceTimersByTime(100);
    expect(resizeCalls()).toBe(1);

    vi.advanceTimersByTime(100);
    expect(resizeCalls()).toBe(1);

    stop();
    Object.defineProperty(window, "innerWidth", { configurable: true, value: previous });
  });
});

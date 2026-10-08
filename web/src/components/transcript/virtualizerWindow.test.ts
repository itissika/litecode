import { elementScroll, observeElementOffset, observeElementRect, Virtualizer } from "@tanstack/react-virtual";
import { afterEach, describe, expect, it } from "vitest";

import {
  followScrollerWindow,
  observeRowHeight,
  rebindVirtualizerWindow,
} from "./virtualizerWindow";

const cleanups: Array<() => void> = [];

afterEach(() => {
  for (const cleanup of cleanups.splice(0)) cleanup();
});

function frame(): { iframe: HTMLIFrameElement; doc: Document; view: Window } {
  const iframe = document.createElement("iframe");
  document.body.appendChild(iframe);
  const doc = iframe.contentDocument;
  const view = iframe.contentWindow;
  if (!doc || !view) throw new Error("popout document missing");
  cleanups.push(() => iframe.remove());
  return { iframe, doc, view };
}

function virtualizer(scroll: HTMLDivElement) {
  const instance = new Virtualizer<HTMLDivElement, Element>({
    count: 1,
    getScrollElement: () => scroll,
    estimateSize: () => 40,
    observeElementRect,
    observeElementOffset,
    scrollToFn: elementScroll,
  });
  instance._willUpdate();
  return instance;
}

describe("rebindVirtualizerWindow", () => {
  it("points measurement at the window that now owns the scroller", () => {
    const scroll = document.createElement("div");
    document.body.appendChild(scroll);
    cleanups.push(() => scroll.remove());
    const instance = virtualizer(scroll);
    expect(instance.targetWindow).toBe(window);

    const pop = frame();
    let popoutObservers = 0;
    class PopoutResizeObserver {
      observe() {}
      unobserve() {}
      disconnect() {}
      constructor() {
        popoutObservers += 1;
      }
    }
    (pop.view as Window & { ResizeObserver: typeof ResizeObserver }).ResizeObserver =
      PopoutResizeObserver as unknown as typeof ResizeObserver;
    pop.doc.body.appendChild(scroll);

    rebindVirtualizerWindow(instance);

    expect(instance.targetWindow).toBe(pop.view);
    expect(popoutObservers).toBeGreaterThan(0);
  });
});

describe("followScrollerWindow", () => {
  it("reports a move into another document", async () => {
    const scroll = document.createElement("div");
    document.body.appendChild(scroll);
    cleanups.push(() => scroll.remove());
    const pop = frame();
    const seen: Array<Document | null> = [];
    const stop = followScrollerWindow(scroll, () => {
      seen.push(scroll.ownerDocument);
    });
    cleanups.push(stop);

    pop.doc.body.appendChild(scroll);
    await Promise.resolve();
    await new Promise<void>((resolve) => {
      pop.view.requestAnimationFrame(() => resolve());
    });

    expect(seen.at(-1)).toBe(pop.doc);
  });
});

describe("observeRowHeight", () => {
  it("uses the resize observer of the window that contains the row", () => {
    const pop = frame();
    const item = pop.doc.createElement("div");
    item.dataset.index = "2";
    pop.doc.body.appendChild(item);
    let constructed = 0;
    class PopoutResizeObserver {
      observe() {}
      unobserve() {}
      disconnect() {}
      constructor() {
        constructed += 1;
      }
    }
    (pop.view as Window & { ResizeObserver: typeof ResizeObserver }).ResizeObserver =
      PopoutResizeObserver as unknown as typeof ResizeObserver;

    const heights: number[] = [];
    const stop = observeRowHeight(item, (height) => heights.push(height));
    cleanups.push(stop);

    expect(constructed).toBe(1);
    expect(heights).toEqual([item.offsetHeight]);
  });
});

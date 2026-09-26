import { describe, expect, it, vi } from "vitest";
import {
  Virtualizer,
  elementScroll,
  observeElementOffset,
  observeElementRect,
} from "@tanstack/virtual-core";

class ResizeObserverStub {
  observe() {}
  unobserve() {}
  disconnect() {}
}
vi.stubGlobal("ResizeObserver", ResizeObserverStub);

/**
 * Prepend keeps the reader on the same bubble when every item key is a real
 * message. The loader used to occupy index 0 with a key that never moves, so
 * this adjustment was a no-op and the page jumped by the whole estimate.
 */
describe("virtualizer prepend anchor", () => {
  it("keeps the same offset inside the anchored bubble after items are prepended", () => {
    const el = document.createElement("div");
    let top = 0;
    Object.defineProperties(el, {
      scrollHeight: { configurable: true, value: 8000 },
      clientHeight: { configurable: true, value: 400 },
      offsetHeight: { configurable: true, value: 400 },
      offsetWidth: { configurable: true, value: 320 },
      scrollTop: {
        configurable: true,
        get: () => top,
        set: (value: number) => {
          top = value;
        },
      },
    });
    el.scrollTo = ((opts?: ScrollToOptions) => {
      top = opts?.top ?? 0;
    }) as typeof el.scrollTo;

    const keys = ["10", "11"];
    const sizes = [88, 240];
    // 40px into the second bubble.
    const intoItem = 40;
    const options = (itemKeys: string[], itemSizes: number[]) => ({
      count: itemKeys.length,
      getScrollElement: () => el,
      estimateSize: (index: number) => itemSizes[index] ?? 100,
      getItemKey: (index: number) => itemKeys[index] ?? index,
      overscan: 6,
      anchorTo: "end" as const,
      observeElementRect,
      observeElementOffset,
      scrollToFn: elementScroll,
      initialOffset: sizes[0]! + intoItem,
    });

    const virtualizer = new Virtualizer(options(keys, sizes));
    virtualizer._willUpdate();

    virtualizer.setOptions(
      options(["0", "1", "10", "11"], [88, 240, 88, 240]),
    );
    virtualizer._willUpdate();

    expect(el.scrollTop).toBe(88 + 240 + 88 + intoItem);
  });
});

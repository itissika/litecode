import { describe, expect, it } from "vitest";

import { shouldCompensateSizeChange } from "./useTranscriptViewport";

describe("shouldCompensateSizeChange", () => {
  // The list is absolutely positioned, so Chromium's scroll anchoring never
  // fires for it (verified: the same growth moved scrollTop in a normal-flow
  // scroller and left it untouched for an abspos item). This predicate is the
  // only compensation the list gets, so the cases below are the whole contract.

  it("does not compensate a newly loaded item below an unpinned viewport", () => {
    // A newly appended bubble below the reader may grow after mount. Shifting
    // scrollTop by that growth pulls the reader toward the latest message.
    expect(
      shouldCompensateSizeChange({
        stickToEnd: false,
        itemEnd: 4000,
        scrollOffset: 2000,
      }),
    ).toBe(false);
  });

  it("compensates a first measurement entirely above an unpinned viewport", () => {
    // History paging can prepend unmeasured bubbles. Their actual sizes move
    // everything below them, so preserve the reader's current content anchor.
    expect(
      shouldCompensateSizeChange({
        stickToEnd: false,
        itemEnd: 1999,
        scrollOffset: 2000,
      }),
    ).toBe(true);
  });

  it("leaves an item overlapping the viewport alone while unpinned", () => {
    // Streamed growth / a FoldCard opening changes the bottom of the item the
    // reader is looking at; compensating by the full delta pushes their view
    // down once per flush and once per 240ms animation frame.
    expect(
      shouldCompensateSizeChange({
        stickToEnd: false,
        itemEnd: 2600,
        scrollOffset: 2000,
      }),
    ).toBe(false);
  });

  it("compensates a size change of an item entirely above the viewport", () => {
    expect(
      shouldCompensateSizeChange({
        stickToEnd: false,
        itemEnd: 2000,
        scrollOffset: 2000,
      }),
    ).toBe(true);
    expect(
      shouldCompensateSizeChange({
        stickToEnd: false,
        itemEnd: 1999,
        scrollOffset: 2000,
      }),
    ).toBe(true);
  });

  it("always compensates while pinned to the end", () => {
    expect(
      shouldCompensateSizeChange({
        stickToEnd: true,
        itemEnd: 2600,
        scrollOffset: 2000,
      }),
    ).toBe(true);
  });
});

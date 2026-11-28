import { describe, expect, it } from "vitest";

import type { Bubble } from "../../lib/transcriptProjection";
import {
  HEAD_TRIM_KEEP_USERS,
  HEAD_TRIM_TRIGGER_USERS,
  selectHeadTrimSeq,
  shouldCompensateSizeChange,
} from "./useTranscriptViewport";

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

function makeTurns(userTurns: number): Bubble[] {
  const out: Bubble[] = [];
  for (let i = 0; i < userTurns; i++) {
    const uSeq = i * 2;
    const aSeq = uSeq + 1;
    const uRow = {
      seq: uSeq,
      kind: "item/user" as const,
      state: "final" as const,
      body: { type: "message" as const, role: "user" as const, content: `u${i}` },
    };
    const aRow = {
      seq: aSeq,
      kind: "item/assistant" as const,
      state: "final" as const,
      body: { type: "message" as const, role: "assistant" as const, content: `a${i}` },
    };
    out.push({
      key: String(uSeq),
      rows: [uRow],
      first: uRow,
      isUser: true,
      markOnly: false,
      followedByUser: false,
      followedByMark: false,
    });
    out.push({
      key: String(aSeq),
      rows: [aRow],
      first: aRow,
      isUser: false,
      markOnly: false,
      followedByUser: i + 1 < userTurns,
      followedByMark: false,
    });
  }
  return out;
}

describe("selectHeadTrimSeq", () => {
  it("stays idle below trigger threshold or while unpinned / loading", () => {
    const below = makeTurns(HEAD_TRIM_TRIGGER_USERS - 1);
    expect(
      selectHeadTrimSeq({
        bubbles: below,
        stickToEnd: true,
        loadingHistory: false,
        firstVisibleIndex: below.length - 2,
      }),
    ).toBeNull();

    const enough = makeTurns(HEAD_TRIM_TRIGGER_USERS);
    expect(
      selectHeadTrimSeq({
        bubbles: enough,
        stickToEnd: false,
        loadingHistory: false,
        firstVisibleIndex: enough.length - 2,
      }),
    ).toBeNull();
    expect(
      selectHeadTrimSeq({
        bubbles: enough,
        stickToEnd: true,
        loadingHistory: true,
        firstVisibleIndex: enough.length - 2,
      }),
    ).toBeNull();
  });

  it("cuts on a user boundary keeping HEAD_TRIM_KEEP_USERS when pinned at tail", () => {
    const bubbles = makeTurns(HEAD_TRIM_TRIGGER_USERS);
    const cutSeq = selectHeadTrimSeq({
      bubbles,
      stickToEnd: true,
      loadingHistory: false,
      firstVisibleIndex: bubbles.length - 2,
    });
    const expectedTurn = HEAD_TRIM_TRIGGER_USERS - HEAD_TRIM_KEEP_USERS;
    expect(cutSeq).toBe(expectedTurn * 2);
  });

  it("clamps cut above firstVisibleIndex so trim cannot trigger loadMore thrash", () => {
    const bubbles = makeTurns(HEAD_TRIM_TRIGGER_USERS);
    // Viewport starts at bubble 8 -> maxCutIdx = 6 (user turn 3, seq 6).
    const cutSeq = selectHeadTrimSeq({
      bubbles,
      stickToEnd: true,
      loadingHistory: false,
      firstVisibleIndex: 8,
    });
    expect(cutSeq).toBe(6);

    // Viewport starts at bubble 4 -> maxCutIdx = 2 (only 1 dropped user turn < min drop 2).
    expect(
      selectHeadTrimSeq({
        bubbles,
        stickToEnd: true,
        loadingHistory: false,
        firstVisibleIndex: 4,
      }),
    ).toBeNull();
  });

  it("skips trim if an edited user bubble would be evicted", () => {
    const bubbles = makeTurns(HEAD_TRIM_TRIGGER_USERS);
    expect(
      selectHeadTrimSeq({
        bubbles,
        stickToEnd: true,
        loadingHistory: false,
        firstVisibleIndex: bubbles.length - 2,
        editingBubbleKey: "0",
      }),
    ).toBeNull();
  });
});

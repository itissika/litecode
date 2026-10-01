import { describe, expect, it } from "vitest";

import type { HumanRow } from "../api/types";
import { isCompactCutRow, projectionRowKey } from "../api/adapter";
import {
  bubbleIdentity,
  canRevertFiles,
  estimateUserBubbleHeight,
  groupRowsForBubbles,
  locateBashTool,
  locateSeq,
  projectBubbles,
  trimPartialHead,
} from "./transcriptProjection";

const userRow: HumanRow = {
  seq: 0,
  kind: "item/user",
  state: "final",

  body: {
    type: "message",
    role: "user",
    id: "msg_user",
    status: "completed",
    content: [{ type: "input_text", text: "hi" }],
  },
};

const liveReasoning: HumanRow = {
  seq: 1,
  kind: "item/assistant",
  state: "final",
  body: {
    type: "reasoning",
    id: "rs_1",
    summary: [{ type: "summary_text", text: "thinking" }],
    content: [{ type: "reasoning_text", text: "thinking" }],
    status: "in_progress",
  },
};

const liveTool: HumanRow = {
  seq: 2,
  kind: "item/tool_call",
  state: "final",
  body: {
    type: "function_call",
    id: "fc_1",
    call_id: "call_1",
    name: "grep",
    arguments: '{"pattern":"foo"}',
    status: "in_progress",
  },
};

const sealedTool: HumanRow = {
  seq: 2,
  kind: "item/tool_call",
  state: "final",
  body: {
    type: "function_call",
    id: "fc_1",
    call_id: "call_1",
    name: "grep",
    arguments: '{"pattern":"foo"}',
    status: "completed",
  },
};

describe("bubbleIdentity", () => {
  it("keys each bubble by min(seq) in the group", () => {
    const before = groupRowsForBubbles([userRow, liveReasoning, liveTool]);
    const after = groupRowsForBubbles([userRow, sealedTool, liveReasoning]);

    expect(before).toHaveLength(2);
    expect(after).toHaveLength(2);
    expect(bubbleIdentity(after, 1)).toBe(bubbleIdentity(before, 1));
    expect(bubbleIdentity(before, 0)).toBe(String(userRow.seq));
    expect(bubbleIdentity(before, 1)).toBe(String(liveReasoning.seq));
  });

  it("does not use a later assistant seq as the virtual key", () => {
    const grouped = groupRowsForBubbles([userRow, liveReasoning, liveTool]);
    expect(bubbleIdentity(grouped, 1)).not.toBe(projectionRowKey(liveTool));
  });

  it("gives reminder and following assistant distinct min(seq) keys", () => {
    const reminder: HumanRow = {
      seq: 9,
      kind: "item/user",
      state: "final",

      body: {
        type: "message",
        role: "user",
        id: "msg_reminder",
        status: "completed",
        content: [
          {
            type: "input_text",
            text: "<system-reminder>\nThe user stopped background bash bg_a (Kill).\n</system-reminder>",
          },
        ],
      },
    };
    const grouped = groupRowsForBubbles([userRow, reminder, liveReasoning]);
    expect(grouped).toHaveLength(3);
    expect(bubbleIdentity(grouped, 1)).toBe(String(reminder.seq));
    expect(bubbleIdentity(grouped, 2)).toBe(String(liveReasoning.seq));
  });

  it("skips unknown kinds instead of folding them into assistant bubbles", () => {
    const unknown = {
      seq: 1,
      kind: "future/widget",
      state: "final",
      body: {
        type: "message",
        role: "assistant",
        id: "ghost",
        status: "completed",
        content: [
          { type: "output_text", text: "do not render", annotations: [] },
        ],
      },
    } as unknown as HumanRow;
    const grouped = groupRowsForBubbles([userRow, unknown, liveReasoning]);
    expect(grouped).toHaveLength(2);
    expect(grouped.flat().map((r) => r.kind)).toEqual([
      "item/user",
      "item/assistant",
    ]);
  });

  it("does not collide keys across a reminder split", () => {
    const reminder: HumanRow = {
      seq: 9,
      kind: "item/user",
      state: "final",

      body: {
        type: "message",
        role: "user",
        id: "msg_reminder",
        status: "completed",
        content: [
          {
            type: "input_text",
            text: "<system-reminder>\nThe user stopped background bash bg_a (Kill).\n</system-reminder>",
          },
        ],
      },
    };
    const grouped = groupRowsForBubbles([
      userRow,
      liveReasoning,
      reminder,
      liveTool,
    ]);
    const keys = grouped.map((_, i) => bubbleIdentity(grouped, i));
    expect(keys).toEqual([
      String(userRow.seq),
      String(liveReasoning.seq),
      String(reminder.seq),
      String(liveTool.seq),
    ]);
    expect(new Set(keys).size).toBe(keys.length);
  });
});

const compactCut = (seq: number): HumanRow => ({
  seq,
  kind: "compacted",
  state: "final",
  body: { summary: "hidden", from: 0, to: seq },
});

describe("groupRowsForBubbles compact cut", () => {
  it("renders the replace event as its own barrier, not the next user bubble", () => {
    const grouped = groupRowsForBubbles([compactCut(0), userRow]);
    expect(grouped).toHaveLength(2);
    expect(grouped[0]?.every(isCompactCutRow)).toBe(true);
    expect(grouped[1]?.map((r) => r.seq)).toEqual([userRow.seq]);
    expect(bubbleIdentity(grouped, 0)).toBe("0");
    expect(bubbleIdentity(grouped, 1)).toBe(String(userRow.seq));
  });

  it("does not push a cut into the previous assistant bubble", () => {
    const grouped = groupRowsForBubbles([
      liveReasoning,
      compactCut(5),
      liveTool,
    ]);
    expect(grouped).toHaveLength(3);
    expect(grouped[1]?.every(isCompactCutRow)).toBe(true);
    expect(grouped[0]?.map((r) => r.seq)).toEqual([liveReasoning.seq]);
    expect(grouped[2]?.map((r) => r.seq)).toEqual([liveTool.seq]);
    expect(bubbleIdentity(grouped, 0)).toBe(String(liveReasoning.seq));
    expect(bubbleIdentity(grouped, 1)).toBe("5");
  });
});

describe("canRevertFiles", () => {
  it("hides the button when no patch max is known", () => {
    expect(canRevertFiles(0, null)).toBe(false);
    expect(canRevertFiles(0, undefined)).toBe(false);
  });

  it("shows when this user row's stem is at or before the latest file patch", () => {
    // stem is user seq + 1 (next_seq at turn start)
    expect(canRevertFiles(0, 1)).toBe(true);
    expect(canRevertFiles(4, 5)).toBe(true);
    expect(canRevertFiles(0, 5)).toBe(true);
    expect(canRevertFiles(5, 5)).toBe(false);
  });
});

describe("locateBashTool", () => {
  it("returns the assistant bubble and process+tool fold ids", () => {
    const bashRow: HumanRow = {
      seq: 3,
      kind: "item/tool_call",
      state: "final",
      body: {
        type: "function_call",
        id: "fc_bash",
        call_id: "c1",
        name: "bash",
        arguments: JSON.stringify({ command: "sleep 1" }),
        status: "in_progress",
      },
    };
    const bubbles = projectBubbles([userRow, liveReasoning, bashRow]);
    const found = locateBashTool(bubbles, "c1", "session-1");
    expect(found?.bubbleIndex).toBe(1);
    expect(found?.foldIds[0]).toMatch(/:process:0$/);
    expect(found?.foldIds[1]).toMatch(/:tool:c1$/);
  });
});

describe("locateSeq", () => {
  it("finds a user seq in its own bubble", () => {
    const bubbles = projectBubbles([userRow, liveReasoning, liveTool]);
    expect(locateSeq(bubbles, 0)).toBe(0);
  });

  it("finds merged assistant rows in the same bubble", () => {
    const bubbles = projectBubbles([userRow, liveReasoning, liveTool]);
    expect(locateSeq(bubbles, 1)).toBe(1);
    expect(locateSeq(bubbles, 2)).toBe(1);
    expect(locateSeq(bubbles, 99)).toBeNull();
  });
});

function assistantRow(seq: number): HumanRow {
  return {
    seq,
    kind: "item/assistant",
    state: "final",
    body: {
      type: "message",
      role: "assistant",
      id: `a${seq}`,
      status: "completed",
      content: [{ type: "output_text", text: "x", annotations: [] }],
    },
  };
}

function userAt(seq: number): HumanRow {
  return { ...userRow, seq };
}

describe("trimPartialHead", () => {
  it("keeps a window that starts at seq 0", () => {
    const rows = [assistantRow(0), userAt(1)];
    expect(trimPartialHead(rows, 0)).toBe(rows);
  });

  it("drops the assistant fragment before the first barrier", () => {
    expect(
      trimPartialHead([assistantRow(40), assistantRow(41), userAt(42)], 40).map(
        (row) => row.seq,
      ),
    ).toEqual([42]);
  });

  it("renders nothing when the window has no barrier", () => {
    expect(trimPartialHead([assistantRow(40), assistantRow(41)], 40)).toEqual(
      [],
    );
  });

  it("keeps the window when it already starts on a barrier", () => {
    const rows = [userAt(40), assistantRow(41)];
    expect(trimPartialHead(rows, 40)).toBe(rows);
  });
});

describe("projectBubbles across a split assistant run", () => {
  it("keeps keys of bubbles that were already visible after the missing head arrives", () => {
    const before = projectBubbles(
      trimPartialHead(
        [assistantRow(40), assistantRow(41), userAt(42), assistantRow(43)],
        40,
      ),
    ).map((bubble) => bubble.key);
    expect(before).toEqual(["42", "43"]);

    const after = projectBubbles(
      trimPartialHead(
        [
          userAt(30),
          assistantRow(39),
          assistantRow(40),
          assistantRow(41),
          userAt(42),
          assistantRow(43),
        ],
        30,
      ),
    ).map((bubble) => bubble.key);
    expect(after).toEqual(["30", "39", "42", "43"]);
    for (const key of before) expect(after).toContain(key);
  });

  it("marks whether the next bubble is a user message", () => {
    const bubbles = projectBubbles([userAt(1), assistantRow(2), userAt(3)]);
    expect(bubbles[1]?.followedByUser).toBe(true);
    expect(bubbles[0]?.followedByUser).toBe(false);
  });
});

describe("estimateUserBubbleHeight", () => {
  it("counts wrapped lines inside the 72ch measure", () => {
    expect(estimateUserBubbleHeight("hi")).toBe(32 + 25);
    expect(estimateUserBubbleHeight("a\nb")).toBe(32 + 50);
    expect(estimateUserBubbleHeight("x".repeat(73))).toBe(32 + 50);
    expect(estimateUserBubbleHeight("", 1)).toBe(32 + 128);
    expect(estimateUserBubbleHeight("hi", 2)).toBe(32 + 25 + 128);
  });
});

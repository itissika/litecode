import { describe, expect, it } from "vitest";

import type { ChatRow } from "../api/adapter";
import { isCompactCutRow, projectionRowKey } from "../api/adapter";
import { bubbleIdentity, canRevertFiles, groupRowsForBubbles } from "./MessageList";

const userRow: ChatRow = {
  id: "item-session-1-0",
  item: {
    type: "message",
    role: "user",
    id: "msg_user",
    status: "completed",
    content: [{ type: "input_text", text: "hi" }],
  },
};

const liveReasoning: ChatRow = {
  id: "live-rs_1",
  streaming: true,
  item: {
    type: "reasoning",
    id: "rs_1",
    summary: [{ type: "summary_text", text: "thinking" }],
    content: [{ type: "reasoning_text", text: "thinking" }],
    status: "in_progress",
  },
};

const liveTool: ChatRow = {
  id: "live-fc_1",
  streaming: true,
  item: {
    type: "function_call",
    id: "fc_1",
    call_id: "call_1",
    name: "grep",
    arguments: "{\"pattern\":\"foo\"}",
    status: "in_progress",
  },
};

const sealedTool: ChatRow = {
  id: "item-session-1-2",
  streaming: false,
  item: {
    type: "function_call",
    id: "fc_1",
    call_id: "call_1",
    name: "grep",
    arguments: "{\"pattern\":\"foo\"}",
    status: "completed",
  },
};

describe("bubbleIdentity", () => {
  it("keeps the assistant bubble key when a later tool seals first", () => {
    const before = groupRowsForBubbles([userRow, liveReasoning, liveTool]);
    const after = groupRowsForBubbles([userRow, sealedTool, liveReasoning]);

    expect(before).toHaveLength(2);
    expect(after).toHaveLength(2);
    expect(bubbleIdentity(after, 1)).toBe(bubbleIdentity(before, 1));
    expect(bubbleIdentity(before, 1)).toBe(
      `assistant-after:user:${projectionRowKey(userRow)}`,
    );
  });

  it("does not use the leading assistant row as the virtual key", () => {
    const grouped = groupRowsForBubbles([userRow, liveReasoning, liveTool]);
    expect(bubbleIdentity(grouped, 1)).not.toBe(projectionRowKey(liveReasoning));
    expect(bubbleIdentity(grouped, 1)).not.toBe(projectionRowKey(liveTool));
  });

  it("keys a user bubble by the user row itself", () => {
    const grouped = groupRowsForBubbles([userRow, liveReasoning]);
    expect(bubbleIdentity(grouped, 0)).toBe(`user:${projectionRowKey(userRow)}`);
  });
});

const compactCut = (id: string): ChatRow => ({
  id,
  kind: "compact_checkpoint",
  item: {
    type: "message",
    role: "user",
    id: `sum_${id}`,
    status: "completed",
    content: [{ type: "input_text", text: "[Conversation summary]\nhidden" }],
  },
});

describe("groupRowsForBubbles compact cut", () => {
  it("does not turn the checkpoint into its own bubble", () => {
    const grouped = groupRowsForBubbles([compactCut("c0"), userRow]);
    expect(grouped).toHaveLength(1);
    expect(grouped[0]?.map((r) => r.id)).toEqual(["c0", userRow.id]);
    expect(bubbleIdentity(grouped, 0)).toBe(`user:${projectionRowKey(userRow)}`);
  });

  it("keeps a cut between assistant items inside one bubble", () => {
    const grouped = groupRowsForBubbles([
      liveReasoning,
      compactCut("c-mid"),
      liveTool,
    ]);
    expect(grouped).toHaveLength(1);
    expect(grouped[0]?.some(isCompactCutRow)).toBe(true);
    expect(bubbleIdentity(grouped, 0)).toBe("assistant-lead");
  });
});

describe("canRevertFiles", () => {
  it("hides the button when no patch max is known", () => {
    expect(canRevertFiles(0, null)).toBe(false);
    expect(canRevertFiles(0, undefined)).toBe(false);
  });

  it("shows on this and earlier user anchors, not later ones", () => {
    expect(canRevertFiles(0, 1)).toBe(true);
    expect(canRevertFiles(1, 1)).toBe(true);
    expect(canRevertFiles(2, 1)).toBe(false);
  });
});

import { describe, expect, it } from "vitest";

import {
  applyStreamEvent,
  bufferItemId,
  deriveUserAnchorK,
  extractBufferIndex,
  isAssistantMessage,
  isChatUserMessage,
  isHumanUserRow,
  isStreamFailureEvent,
  isSystemReminderItem,
  itemAuthorityId,
  itemPlainText,
  liveItemRowId,
  markFunctionCallsFailed,
  projectionRowKey,
  sealMismatchError,
  sealProjectionRow,
  userTextItem,
} from "./adapter";
import type { Item, ResponseStreamEvent } from "./types";

describe("deriveUserAnchorK", () => {
  it("counts only user messages and adds the load baseline", () => {
    const tool: Item = {
      type: "function_call",
      call_id: "c1",
      name: "bash",
      arguments: "{}",
    };
    const assistant: Item = {
      type: "message",
      role: "assistant",
      id: "a1",
      status: "completed",
      content: [{ type: "output_text", text: "ok", annotations: [] }],
    };
    const messages = [
      { item: userTextItem("u0") },
      { item: tool },
      { item: assistant },
      { item: userTextItem("u1") },
      { item: userTextItem("u2") },
    ];
    expect(deriveUserAnchorK(messages, 0, 0)).toBe(0);
    expect(deriveUserAnchorK(messages, 3, 0)).toBe(1);
    expect(deriveUserAnchorK(messages, 4, 0)).toBe(2);
    expect(deriveUserAnchorK(messages, 3, 5)).toBe(6);
  });

  // REV-11 hard gate: the FE k derivation must equal the backend
  // `entry_user_detail_count` on the SAME fixture as the backend test
  // `revert_contract_three_states` (tests/stage_a_ctx_consistency.rs). That
  // fixture yields: pre-compact u0,u1 → compact checkpoint ("summary") →
  // post-compact u2,u3, with `entry_user_detail_count() == 2`. The compact
  // checkpoint is a user role but `kind='compact_checkpoint'`, so it must NOT
  // be counted as a revert anchor.
  it("REV-11: derives the backend user_detail_count on the revert_contract_three_states fixture", () => {
    const messages = [
      // The buffer window the FE receives includes the compact checkpoint row
      // (kind='compact_checkpoint'), which the backend excludes from the count.
      { item: userTextItem("summary"), kind: "compact_checkpoint" },
      { item: userTextItem("u2") },
      { item: userTextItem("u3") },
    ];
    // backend `entry_user_detail_count()` == 2 for this fixture.
    const entryUserDetailCount = 2;
    const derived = deriveUserAnchorK(messages, messages.length, 0);
    expect(derived).toBe(entryUserDetailCount);
    // Anchor for the last visible user (u3, buffer index 2) is k=1 (0-based,
    // checkpoint excluded).
    expect(deriveUserAnchorK(messages, 2, 0)).toBe(1);
  });
});

describe("stream failure invalidation (FE-06)", () => {
  it("classifies turn-level failure events", () => {
    expect(isStreamFailureEvent({ type: "response.failed", response: {} })).toBe(true);
    expect(isStreamFailureEvent({ type: "response.incomplete", response: {} })).toBe(false);
    expect(isStreamFailureEvent({ type: "error", code: "internal", message: "x" })).toBe(true);
    expect(isStreamFailureEvent({ type: "response.output_text.delta", item_id: "m", delta: "hi" })).toBe(false);
  });

  it("marks in_progress function calls failed and leaves others untouched", () => {
    const items: Item[] = [
      {
        type: "function_call",
        id: "fc_1",
        call_id: "call_1",
        name: "bash",
        arguments: "{}",
        status: "in_progress",
      },
      {
        type: "function_call",
        id: "fc_2",
        call_id: "call_2",
        name: "read",
        arguments: "{}",
        status: "completed",
      },
      {
        type: "message",
        role: "assistant",
        id: "m1",
        status: "in_progress",
        content: [{ type: "output_text", text: "hi", annotations: [] }],
      },
    ];
    const next = markFunctionCallsFailed(items);
    expect(next[0]).toMatchObject({ status: "failed" });
    expect(next[1]).toMatchObject({ status: "completed" });
    expect(next[2]).toEqual(items[2]);
  });
});

describe("buffer item ids", () => {
  it("roundtrips buffer index", () => {
    const id = bufferItemId("sess-abc", 12);
    expect(extractBufferIndex(id)).toBe(12);
  });

  it("ignores live ids", () => {
    expect(extractBufferIndex(liveItemRowId("msg_1"))).toBeNull();
  });
});

describe("projectionRowKey", () => {
  it("stays stable across live→buffer seal when Item authority id is present", () => {
    const item: Item = {
      type: "reasoning",
      id: "rs_1",
      summary: [{ type: "summary_text", text: "plan" }],
      content: [{ type: "reasoning_text", text: "plan" }],
      status: "completed",
    };
    const liveKey = projectionRowKey({
      id: liveItemRowId("rs_1"),
      item,
      streaming: true,
    });
    const sealedKey = projectionRowKey({
      id: bufferItemId("sess", 3),
      item,
      streaming: false,
    });
    expect(liveKey).toBe("rs_1");
    expect(sealedKey).toBe("rs_1");
    expect(liveKey).toBe(sealedKey);
  });

  it("uses call_id for function_call when item.id is absent", () => {
    const item: Item = {
      type: "function_call",
      call_id: "call_1",
      name: "bash",
      arguments: "{}",
    };
    expect(
      projectionRowKey({ id: liveItemRowId("call_1"), item, streaming: true }),
    ).toBe("call_1");
  });
});

describe("userTextItem", () => {
  it("builds Responses input message Item", () => {
    const item = userTextItem("hello");
    expect(item).toEqual({
      type: "message",
      role: "user",
      content: [{ type: "input_text", text: "hello" }],
    });
    expect(itemPlainText(item)).toBe("hello");
  });
});

describe("isSystemReminderItem", () => {
  it("detects wrapped auto-turn input and excludes it from chat-user bubbles", () => {
    const reminder = userTextItem(
      "<system-reminder>\nThe user stopped background bash bg_a (Kill).\n</system-reminder>",
    );
    expect(isSystemReminderItem(reminder)).toBe(true);
    expect(isChatUserMessage(reminder)).toBe(false);
    expect(isChatUserMessage(userTextItem("hello"))).toBe(true);
    expect(isSystemReminderItem(userTextItem("hello"))).toBe(false);
    expect(
      isHumanUserRow({ item: reminder, kind: "detail" }),
    ).toBe(false);
    expect(
      isHumanUserRow({ item: userTextItem("hello") }),
    ).toBe(true);
    expect(
      isHumanUserRow({
        item: userTextItem("compact summary"),
        kind: "compact_checkpoint",
      }),
    ).toBe(false);
  });
});

describe("applyStreamEvent — Item-shaped accumulation", () => {
  it("accumulates response.output_text.delta onto a message Item", () => {
    let item: Item | undefined;
    const r1 = applyStreamEvent(item, {
      type: "response.output_text.delta",
      sequence_number: 1,
      item_id: "msg_1",
      output_index: 0,
      content_index: 0,
      delta: "hi",
    });
    expect(r1.kind).toBe("upsert");
    if (r1.kind !== "upsert") return;
    item = r1.item;
    const r2 = applyStreamEvent(item, {
      type: "response.output_text.delta",
      sequence_number: 2,
      item_id: "msg_1",
      output_index: 0,
      content_index: 0,
      delta: " there",
    });
    expect(r2.kind).toBe("upsert");
    if (r2.kind !== "upsert") return;
    expect(r2.itemId).toBe("msg_1");
    expect(itemPlainText(r2.item)).toBe("hi there");
    expect(itemAuthorityId(r2.item)).toBe("msg_1");
  });

  it("accumulates response.reasoning_text.delta onto a reasoning Item", () => {
    let item: Item | undefined;
    const r1 = applyStreamEvent(item, {
      type: "response.reasoning_text.delta",
      item_id: "rs_1",
      delta: "think ",
    });
    expect(r1.kind).toBe("upsert");
    if (r1.kind !== "upsert") return;
    item = r1.item;
    const r2 = applyStreamEvent(item, {
      type: "response.reasoning_text.delta",
      item_id: "rs_1",
      delta: "hard",
    });
    expect(r2.kind).toBe("upsert");
    if (r2.kind !== "upsert") return;
    expect(itemPlainText(r2.item)).toBe("think hard");
    expect(r2.item.type).toBe("reasoning");
  });

  it("accumulates function_call_arguments.delta onto a function_call Item", () => {
    const r1 = applyStreamEvent(undefined, {
      type: "response.function_call_arguments.delta",
      item_id: "fc_1",
      delta: '{"x":',
    });
    expect(r1.kind).toBe("upsert");
    if (r1.kind !== "upsert") return;
    const r2 = applyStreamEvent(r1.item, {
      type: "response.function_call_arguments.done",
      item_id: "fc_1",
      arguments: '{"x":1}',
      name: "bash",
    });
    expect(r2.kind).toBe("upsert");
    if (r2.kind !== "upsert") return;
    expect(r2.item).toMatchObject({
      type: "function_call",
      id: "fc_1",
      call_id: "fc_1",
      name: "bash",
      arguments: '{"x":1}',
    });
  });

  it("applies Responses tool_call fixture order: name then arguments", () => {
    // Mirrors tests/fixtures/sse/responses/tool_call.txt event order.
    const events: ResponseStreamEvent[] = [
      {
        type: "response.output_item.added",
        sequence_number: 1,
        output_index: 0,
        item: {
          type: "function_call",
          id: "fc_read_1",
          call_id: "call_read_1",
          name: "read",
          arguments: "",
          status: "in_progress",
        },
      },
      {
        type: "response.function_call_arguments.delta",
        sequence_number: 2,
        item_id: "fc_read_1",
        output_index: 0,
        delta: '{"path":"test.txt"}',
      },
    ];

    let item: Item | undefined;
    const r0 = applyStreamEvent(item, events[0]);
    expect(r0.kind).toBe("upsert");
    if (r0.kind !== "upsert") return;
    expect(r0.item).toMatchObject({ name: "read", arguments: "" });
    item = r0.item;

    const r1 = applyStreamEvent(item, events[1]);
    expect(r1.kind).toBe("upsert");
    if (r1.kind !== "upsert") return;
    expect(r1.item).toMatchObject({
      name: "read",
      arguments: '{"path":"test.txt"}',
    });
  });

  it("no-ops output_item.added for non-function_call items", () => {
    const next = applyStreamEvent(undefined, {
      type: "response.output_item.added",
      item: {
        type: "message",
        id: "msg_1",
        role: "assistant",
        status: "in_progress",
        content: [],
      },
    });
    expect(next).toEqual({ kind: "noop" });
  });

  it("does not clear an existing name when output_item.added has empty name", () => {
    const seeded = applyStreamEvent(undefined, {
      type: "response.output_item.added",
      item: {
        type: "function_call",
        id: "fc_1",
        call_id: "fc_1",
        name: "bash",
        arguments: "",
      },
    });
    expect(seeded.kind).toBe("upsert");
    if (seeded.kind !== "upsert") return;
    const again = applyStreamEvent(seeded.item, {
      type: "response.output_item.added",
      item: {
        type: "function_call",
        id: "fc_1",
        call_id: "fc_1",
        name: "",
        arguments: "",
      },
    });
    expect(again.kind).toBe("upsert");
    if (again.kind !== "upsert") return;
    expect(again.item).toMatchObject({ name: "bash" });
  });

  it("accumulates multiple function_call_arguments.delta chunks before done", () => {
    let item: Item | undefined;
    for (const delta of ['{"path":', '"a.txt"', "}"]) {
      const r = applyStreamEvent(item, {
        type: "response.function_call_arguments.delta",
        item_id: "fc_multi",
        delta,
      });
      expect(r.kind).toBe("upsert");
      if (r.kind !== "upsert") return;
      item = r.item;
    }
    expect(item).toMatchObject({
      type: "function_call",
      id: "fc_multi",
      arguments: '{"path":"a.txt"}',
    });
  });

  it("no-ops documented lifecycle events", () => {
    const next = applyStreamEvent(undefined, {
      type: "response.created",
      response: {},
    });
    expect(next).toEqual({ kind: "noop" });
  });

  it("errors on unhandled semantic event types", () => {
    const next = applyStreamEvent(undefined, {
      type: "response.refusal.delta",
      item_id: "msg_1",
      delta: "no",
    });
    expect(next.kind).toBe("error");
    if (next.kind !== "error") return;
    expect(next.message).toMatch(/unhandled semantic/);
  });

  it("errors when event type contradicts live Item type", () => {
    const msg = applyStreamEvent(undefined, {
      type: "response.output_text.delta",
      sequence_number: 1,
      item_id: "x1",
      output_index: 0,
      content_index: 0,
      delta: "hi",
    });
    expect(msg.kind).toBe("upsert");
    if (msg.kind !== "upsert") return;
    const bad = applyStreamEvent(msg.item, {
      type: "response.reasoning_text.delta",
      item_id: "x1",
      delta: "oops",
    });
    expect(bad.kind).toBe("error");
  });
});

describe("sealMismatchError", () => {
  it("flags type contradiction", () => {
    const live: Item = {
      type: "message",
      role: "assistant",
      id: "msg_1",
      status: "in_progress",
      content: [{ type: "output_text", text: "a", annotations: [] }],
    };
    const committed: Item = {
      type: "reasoning",
      id: "msg_1",
      summary: [],
    };
    expect(sealMismatchError(live, committed)).toMatch(/type=/);
  });

  it("allows same-type seal", () => {
    const live: Item = emptyMsg("msg_1", "partial");
    const committed: Item = emptyMsg("msg_1", "final");
    expect(sealMismatchError(live, committed)).toBeNull();
  });
});

describe("sealProjectionRow", () => {
  it("stamps the slot and keeps content when visible text already matches", () => {
    const content = [{ type: "output_text" as const, text: "hello", annotations: [] }];
    const liveItem: Item = {
      type: "message",
      role: "assistant",
      id: "msg_1",
      status: "in_progress",
      content,
    };
    const { row, mismatch } = sealProjectionRow(
      { id: liveItemRowId("msg_1"), item: liveItem, streaming: true },
      {
        type: "message",
        role: "assistant",
        id: "msg_1",
        status: "completed",
        content: [{ type: "output_text", text: "hello", annotations: [] }],
      },
      bufferItemId("s", 0),
    );
    expect(mismatch).toBeNull();
    expect(row.id).toBe(bufferItemId("s", 0));
    expect(row.streaming).toBe(false);
    expect(row.item).toMatchObject({ id: "msg_1", status: "completed" });
    expect(isAssistantMessage(row.item) && row.item.content).toBe(content);
  });

  it("replaces the item when visible content diverges", () => {
    const liveItem = emptyMsg("msg_1", "partial");
    const committed = emptyMsg("msg_1", "final");
    const { row, mismatch } = sealProjectionRow(
      { id: liveItemRowId("msg_1"), item: liveItem, streaming: true },
      committed,
      bufferItemId("s", 1),
    );
    expect(mismatch).toBeNull();
    expect(row.item).toBe(committed);
  });
});

function emptyMsg(id: string, text: string): Item {
  return {
    type: "message",
    role: "assistant",
    id,
    status: "completed",
    content: [{ type: "output_text", text, annotations: [] }],
  };
}

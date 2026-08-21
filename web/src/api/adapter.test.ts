import { describe, expect, it } from "vitest";

import {
  applyStreamEvent,
  deriveUserAnchorK,
  hydrateUserDetailBefore,
  isAssistantMessage,
  isHiddenHumanRow,
  isHumanUserRow,
  isHumanViewKind,
  isWellFormedBufferRow,
  itemAuthorityId,
  mergeCommittedItem,
  sealMismatchError,
  isStreamFailureEvent,
  itemPlainText,
  markFunctionCallsFailed,
  userTextItem,
} from "./adapter";
import type { HumanRow, Item, ResponseStreamEvent } from "./types";

const userRow = (seq: number, text: string): HumanRow => ({
  seq, kind: "item/user", body: userTextItem(text),
});

describe("kind-based HumanView rows", () => {
  it("counts only explicit item/user rows and hides reminders", () => {
    const rows: HumanRow[] = [
      userRow(0, "u0"),
      { seq: 1, kind: "compacted", body: { summary: "hidden", from: 0, to: 1 } },
      { seq: 2, kind: "reminder/job_exit", body: { reason: "kill", text: "<system-reminder>not inspected</system-reminder>" } },
      userRow(3, "u1"),
    ];
    expect(deriveUserAnchorK(rows, 3, 5)).toBe(6);
    expect(isHiddenHumanRow(rows[2]!)).toBe(true);
    expect(isHumanUserRow(rows[0]!)).toBe(true);
  });

  it("does not infer log kind from body", () => {
    expect(isHumanViewKind("future/widget")).toBe(false);
    expect(
      isWellFormedBufferRow({
        seq: 1,
        item: userTextItem("legacy"),
      }),
    ).toBe(false);
    expect(
      isWellFormedBufferRow({
        seq: 1,
        kind: "item/user",
        body: userTextItem("ok"),
      }),
    ).toBe(true);
  });

  it("hydrates userDetailBefore from the server prefix for partial windows", () => {
    expect(hydrateUserDetailBefore(10, 3, 0)).toBe(3);
    expect(hydrateUserDetailBefore(0, 3, 9)).toBe(0);
    expect(hydrateUserDetailBefore(10, undefined, 2)).toBe(2);
  });

  it("classifies stream failures and invalidates live calls", () => {
    expect(isStreamFailureEvent({ type: "response.failed", response: {} })).toBe(true);
    const next = markFunctionCallsFailed([{ type: "function_call", call_id: "call_1", name: "bash", arguments: "{}", status: "in_progress" }]);
    expect(next[0]).toMatchObject({ status: "failed" });
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

describe("mergeCommittedItem", () => {
  it("stamps status when visible text already matches", () => {
    const content = [{ type: "output_text" as const, text: "hello", annotations: [] }];
    const liveItem: Item = {
      type: "message",
      role: "assistant",
      id: "msg_1",
      status: "in_progress",
      content,
    };
    const merged = mergeCommittedItem(liveItem, {
      type: "message",
      role: "assistant",
      id: "msg_1",
      status: "completed",
      content: [{ type: "output_text", text: "hello", annotations: [] }],
    });
    expect(merged).toMatchObject({ id: "msg_1", status: "completed" });
    expect(isAssistantMessage(merged) && merged.content).toBe(content);
  });

  it("replaces the item when visible content diverges", () => {
    const liveItem = emptyMsg("msg_1", "partial");
    const committed = emptyMsg("msg_1", "final");
    expect(mergeCommittedItem(liveItem, committed)).toBe(committed);
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

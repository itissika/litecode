/**
 * Seq-keyed message store: load/item share one map; deltas only hit an existing seq.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";

import { isCompactCutRow, itemPlainText } from "../api/adapter";
import type { BufferLoaded, Item, WireBufferEvent } from "../api/types";
import { useMessageStore } from "./messageStore";
import { EMPTY_SLICE as EMPTY_TURN, useTurnStore } from "./turnStore";
import { useToastStore } from "./toastStore";

function assistantMsg(id: string, text: string, status: "in_progress" | "completed" = "completed"): Item {
  return {
    type: "message",
    role: "assistant",
    id,
    status,
    content: [{ type: "output_text", text, annotations: [] }],
  };
}

function userMsg(text: string): Item {
  return {
    type: "message",
    role: "user",
    content: [{ type: "input_text", text }],
  };
}

function ev(
  seq: number,
  item: Item,
  extra: Partial<WireBufferEvent> = {},
): WireBufferEvent {
  return {
    seq,
    type: extra.type ?? (item.type === "message" && "role" in item && item.role === "user"
      ? "item/user"
      : "item/assistant"),
    surface_op: extra.surface_op ?? "append",
    item,
    ...extra,
  };
}

function load(sid: string, events: WireBufferEvent[], from = 0, to?: number): void {
  const loaded: BufferLoaded = {
    session_id: sid,
    from_seq: from,
    to_seq: to ?? (events.length ? Math.max(...events.map((e) => e.seq)) + 1 : from),
    events,
  };
  useMessageStore.getState().onBufferLoaded(sid, loaded);
}

function markTurnRunning(sessionId: string, turnId = "t1"): void {
  useTurnStore.setState({
    byId: new Map([[sessionId, { ...EMPTY_TURN, runState: "running", currentTurnId: turnId }]]),
  });
}

describe("messageStore seq map", () => {
  beforeEach(() => {
    useMessageStore.setState({ bySession: new Map() });
    useTurnStore.setState({ byId: new Map() });
    useToastStore.setState({ toasts: [] });
  });

  it("loads events by seq and sorts by seq", () => {
    const sid = "s1";
    load(sid, [
      ev(2, assistantMsg("a", "two")),
      ev(0, userMsg("zero")),
      ev(1, assistantMsg("b", "one")),
    ]);
    const slice = useMessageStore.getState().bySession.get(sid)!;
    expect(slice.messages.map((r) => r.seq)).toEqual([0, 1, 2]);
    expect(slice.messages.map((r) => itemPlainText(r.item))).toEqual(["zero", "one", "two"]);
    expect(slice.bySeq.size).toBe(3);
  });

  it("replace surface_op is a compact cut, not a user anchor", () => {
    const sid = "s-cut";
    load(sid, [
      ev(0, userMsg("ask")),
      ev(1, assistantMsg("a", "old")),
      ev(2, userMsg("summary"), {
        type: "item/user",
        surface_op: { op: "replace", start: 0, end: 2 },
      }),
      ev(3, userMsg("continue")),
    ]);
    const slice = useMessageStore.getState().bySession.get(sid)!;
    expect(slice.messages.filter(isCompactCutRow)).toHaveLength(1);
    expect(isCompactCutRow(slice.messages[2]!)).toBe(true);
  });

  it("rejects load events that omit seq", () => {
    const sid = "s-bad";
    useMessageStore.getState().onBufferLoaded(sid, {
      session_id: sid,
      from_seq: 0,
      to_seq: 1,
      events: [{ type: "item/user", item: userMsg("x") } as never],
    });
    const slice = useMessageStore.getState().bySession.get(sid)!;
    expect(slice.messages).toHaveLength(0);
    expect(slice.shapeError).toMatch(/seq/);
  });

  it("stream delta without a seq mapping does not create a row", () => {
    const sid = "s-live";
    markTurnRunning(sid);
    useMessageStore.getState().applyStreamEvent(sid, "t1", 1, {
      type: "response.output_text.delta",
      sequence_number: 1,
      item_id: "msg_1",
      output_index: 0,
      content_index: 0,
      delta: "ghost",
    });
    expect(useMessageStore.getState().bySession.get(sid)?.messages ?? []).toHaveLength(0);
  });

  it("deltas update the seq allocated by buffer/item while in_progress", () => {
    const sid = "s-delta";
    markTurnRunning(sid);
    useMessageStore.getState().onBufferItem(sid, {
      session_id: sid,
      seq: 4,
      type: "item/assistant",
      surface_op: "append",
      item: assistantMsg("msg_1", "", "in_progress"),
    });
    useMessageStore.getState().applyStreamEvent(sid, "t1", 1, {
      type: "response.output_text.delta",
      sequence_number: 1,
      item_id: "msg_1",
      output_index: 0,
      content_index: 0,
      delta: "hello",
    });
    const row = useMessageStore.getState().bySession.get(sid)!.messages[0]!;
    expect(row.seq).toBe(4);
    expect(itemPlainText(row.item)).toBe("hello");
  });

  it("G4: sealed seq ignores a later delta for the same item_id", () => {
    const sid = "s-g4";
    markTurnRunning(sid, "t-after");
    load(sid, [ev(0, assistantMsg("msg_1", "old reply")), ev(1, userMsg("rolled-up"))]);
    useMessageStore.getState().applyStreamEvent(sid, "t-after", 1, {
      type: "response.output_text.delta",
      sequence_number: 1,
      item_id: "msg_1",
      output_index: 0,
      content_index: 0,
      delta: "new live text",
    });
    const slice = useMessageStore.getState().bySession.get(sid)!;
    const old = slice.messages.find((m) => m.seq === 0)!;
    expect(itemPlainText(old.item)).toBe("old reply");
    expect(slice.messages.some((m) => itemPlainText(m.item).includes("new live text"))).toBe(
      false,
    );
  });

  it("pending user is not a seq key and seals on matching buffer/item", () => {
    const sid = "s-pend";
    useMessageStore.getState().pushPendingUser(sid, {
      clientId: "pending-1",
      item: userMsg("hello"),
    });
    let slice = useMessageStore.getState().bySession.get(sid)!;
    expect(slice.messages).toHaveLength(0);
    expect(slice.pendingUser?.clientId).toBe("pending-1");
    useMessageStore.getState().onBufferItem(sid, {
      session_id: sid,
      seq: 0,
      type: "item/user",
      surface_op: "append",
      item: userMsg("hello"),
    });
    slice = useMessageStore.getState().bySession.get(sid)!;
    expect(slice.pendingUser).toBeNull();
    expect(slice.messages).toHaveLength(1);
    expect(slice.messages[0]!.seq).toBe(0);
  });

  it("second pending user is dropped while one is already waiting", () => {
    const sid = "s-one";
    useMessageStore.getState().pushPendingUser(sid, {
      clientId: "a",
      item: userMsg("one"),
    });
    useMessageStore.getState().pushPendingUser(sid, {
      clientId: "b",
      item: userMsg("two"),
    });
    expect(useMessageStore.getState().bySession.get(sid)!.pendingUser?.clientId).toBe("a");
  });

  it("finalizeTurn does not drop seq rows", () => {
    const sid = "s-fin";
    markTurnRunning(sid);
    useMessageStore.getState().onBufferItem(sid, {
      session_id: sid,
      seq: 1,
      type: "item/assistant",
      surface_op: "append",
      item: assistantMsg("msg_1", "mid", "in_progress"),
    });
    useMessageStore.getState().finalizeTurn(sid, "t1");
    const slice = useMessageStore.getState().bySession.get(sid)!;
    expect(slice.messages).toHaveLength(1);
    expect(slice.messages[0]!.seq).toBe(1);
    expect(slice.messages[0]!.streaming).toBe(false);
  });

  it("buffer/reverted keeps seq < next_seq", () => {
    const sid = "s-rev";
    load(sid, [ev(0, userMsg("a")), ev(1, assistantMsg("x", "b")), ev(2, userMsg("c"))]);
    useMessageStore.getState().onBufferReverted(sid, {
      session_id: sid,
      last_seq: 0,
      next_seq: 1,
    });
    const slice = useMessageStore.getState().bySession.get(sid)!;
    expect(slice.messages.map((r) => r.seq)).toEqual([0]);
    expect(slice.toSeq).toBe(1);
  });
});

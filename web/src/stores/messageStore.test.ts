/**
 * Low-churn business projection: stream upsert then buffer/item seal,
 * including seal mismatch → shapeError (fail-closed: no overwrite).
 */
import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  deriveUserAnchorK,
  isCompactCutRow,
  isFunctionCall,
  isUserMessage,
  itemAuthorityId,
  itemPlainText,
  liveItemRowId,
  rowBufferIndex,
} from "../api/adapter";
import type { BufferLoaded, Item } from "../api/types";
import { useConnectionStore } from "./connectionStore";
import { useMessageStore } from "./messageStore";
import { useToastStore } from "./toastStore";
import { EMPTY_SLICE as EMPTY_TURN, useTurnStore } from "./turnStore";

vi.stubGlobal(
  "window",
  Object.assign(globalThis, {
    setTimeout: (fn: () => void) => {
      fn();
      return 0;
    },
  }),
);

function assistantMsg(id: string, text: string): Item {
  return {
    type: "message",
    role: "assistant",
    id,
    status: "completed",
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

function functionCall(callId: string, name: string): Item {
  return {
    type: "function_call",
    id: callId,
    call_id: callId,
    name,
    arguments: "{}",
    status: "completed",
  };
}

function functionCallOutput(callId: string, output: string): Item {
  return {
    type: "function_call_output",
    call_id: callId,
    output,
  };
}

function markTurnRunning(sessionId: string, turnId = "t1"): void {
  useTurnStore.setState({
    byId: new Map([
      [
        sessionId,
        { ...EMPTY_TURN, runState: "running", currentTurnId: turnId },
      ],
    ]),
  });
}

function loadBuffer(
  sid: string,
  loaded: Omit<BufferLoaded, "indices"> & { indices?: number[] },
): void {
  const indices =
    loaded.indices ?? loaded.items.map((_, i) => loaded.start + i);
  useMessageStore.getState().onBufferLoaded(sid, { ...loaded, indices });
}

describe("messageStore Item projection", () => {
  beforeEach(() => {
    useMessageStore.setState({ bySession: new Map() });
    useTurnStore.setState({ byId: new Map() });
    useToastStore.setState({ toasts: [] });
  });

  it("buffer/item checkpoint appends a cut without rewriting detail indices", () => {
    const sid = "s-compact";
    markTurnRunning(sid);
    loadBuffer(sid, {
      session_id: sid,
      start: 0,
      end: 3,
      items: [
        assistantMsg("rs_old_0", "first"),
        assistantMsg("rs_old_1", "second"),
        assistantMsg("rs_kept", "kept"),
      ],
      kinds: ["detail", "detail", "detail"],
      user_detail_before: 0,
    });
    useMessageStore.getState().applyStreamEvent(sid, "t1", 1, {
      type: "response.output_text.delta",
      sequence_number: 1,
      item_id: "msg_live",
      output_index: 0,
      content_index: 0,
      delta: "streaming",
    });
    const sendRpc = vi.fn();
    useConnectionStore.setState({ sendRpc } as never);

    useMessageStore.getState().onBufferItem(sid, {
      session_id: sid,
      buffer_index: 3,
      item: userMsg("rolled-up"),
      kind: "compact_checkpoint",
    });

    const slice = useMessageStore.getState().bySession.get(sid)!;
    expect(slice.shapeError).toBeNull();
    expect(useToastStore.getState().toasts).toHaveLength(0);
    expect(slice.messages.map((row) => itemPlainText(row.item))).toEqual([
      "first",
      "second",
      "kept",
      "rolled-up",
      "streaming",
    ]);
    expect(rowBufferIndex(slice.messages[0]!)).toBe(0);
    expect(rowBufferIndex(slice.messages[1]!)).toBe(1);
    expect(rowBufferIndex(slice.messages[2]!)).toBe(2);
    expect(slice.messages[3]?.kind).toBe("compact_checkpoint");
    expect(rowBufferIndex(slice.messages[3]!)).toBe(3);
    expect(slice.messages.some((row) => row.id === liveItemRowId("msg_live"))).toBe(
      true,
    );
    expect(sendRpc).not.toHaveBeenCalled();
  });

  it("buffer/item second checkpoint appends without moving earlier rows", () => {
    const sid = "s-compact-2";
    loadBuffer(sid, {
      session_id: sid,
      start: 0,
      end: 4,
      items: [
        assistantMsg("a0", "first"),
        assistantMsg("a1", "second"),
        userMsg("old summary"),
        assistantMsg("a2", "after"),
      ],
      kinds: ["detail", "detail", "compact_checkpoint", "detail"],
      user_detail_before: 0,
    });
    const sendRpc = vi.fn();
    useConnectionStore.setState({ sendRpc } as never);

    useMessageStore.getState().onBufferItem(sid, {
      session_id: sid,
      buffer_index: 4,
      item: userMsg("new summary"),
      kind: "compact_checkpoint",
    });

    const slice = useMessageStore.getState().bySession.get(sid)!;
    expect(slice.messages.map((row) => itemPlainText(row.item))).toEqual([
      "first",
      "second",
      "old summary",
      "after",
      "new summary",
    ]);
    expect(slice.messages.filter(isCompactCutRow)).toHaveLength(2);
    expect(rowBufferIndex(slice.messages[2]!)).toBe(2);
    expect(slice.messages[2]?.kind).toBe("compact_checkpoint");
    expect(rowBufferIndex(slice.messages[3]!)).toBe(3);
    expect(slice.messages[4]?.kind).toBe("compact_checkpoint");
    expect(rowBufferIndex(slice.messages[4]!)).toBe(4);
    expect(sendRpc).not.toHaveBeenCalled();
  });

  it("stream upsert then buffer/item seal keeps same authority id", () => {
    const sid = "s1";
    useMessageStore.getState().applyStreamEvent(sid, "t1", 1, {
      type: "response.output_text.delta",
      sequence_number: 1,
      item_id: "msg_1",
      output_index: 0,
      content_index: 0,
      delta: "hi",
    });
    const mid = useMessageStore.getState().bySession.get(sid)!;
    expect(mid.messages).toHaveLength(1);
    expect(mid.messages[0].id).toBe(liveItemRowId("msg_1"));
    expect(mid.messages[0].streaming).toBe(true);

    useMessageStore.getState().onBufferItem(sid, {
      session_id: sid,
      buffer_index: 0,
      item: assistantMsg("msg_1", "hi there"),
    });
    const sealed = useMessageStore.getState().bySession.get(sid)!;
    expect(sealed.messages).toHaveLength(1);
    expect(sealed.messages[0].id).toBe(liveItemRowId("msg_1"));
    expect(rowBufferIndex(sealed.messages[0])).toBe(0);
    expect(sealed.messages[0].streaming).toBe(false);
    expect(sealed.messages[0].item).toMatchObject({
      type: "message",
      id: "msg_1",
    });
    expect(sealed.shapeError).toBeNull();
  });

  it("ignores late stream deltas after buffer/item seal (no end-char duplication)", () => {
    const sid = "s-seal-race";
    useMessageStore.getState().applyStreamEvent(sid, "t1", 1, {
      type: "response.output_text.delta",
      sequence_number: 1,
      item_id: "msg_race",
      output_index: 0,
      content_index: 0,
      delta: "你好",
    });

    useMessageStore.getState().onBufferItem(sid, {
      session_id: sid,
      buffer_index: 0,
      item: assistantMsg("msg_race", "你好世界"),
    });

    // Simulate rAF flush landing after authority seal (DeepSeek/chat path).
    useMessageStore.getState().applyStreamEvent(sid, "t1", 1, {
      type: "response.output_text.delta",
      sequence_number: 2,
      item_id: "msg_race",
      output_index: 0,
      content_index: 0,
      delta: "世界",
    });

    const slice = useMessageStore.getState().bySession.get(sid)!;
    expect(slice.messages).toHaveLength(1);
    expect(slice.messages[0].streaming).toBe(false);
    expect(slice.messages[0].item).toEqual(assistantMsg("msg_race", "你好世界"));
  });

  it("reorders sealed rows by buffer_index when stream arrival order diverged", () => {
    const sid = "s-order";
    // Message deltas arrive before reasoning → live list is inverted vs transcript.
    useMessageStore.getState().applyStreamEvent(sid, "t1", 1, {
      type: "response.output_text.delta",
      sequence_number: 1,
      item_id: "msg_out",
      output_index: 1,
      content_index: 0,
      delta: "answer",
    });
    useMessageStore.getState().applyStreamEvent(sid, "t1", 1, {
      type: "response.reasoning_text.delta",
      sequence_number: 2,
      item_id: "rs_1",
      output_index: 0,
      content_index: 0,
      delta: "think",
    });
    let slice = useMessageStore.getState().bySession.get(sid)!;
    expect(slice.messages.map((m) => m.item.type)).toEqual(["message", "reasoning"]);

    // Backend commits reasoning then message (buffer order).
    useMessageStore.getState().onBufferItem(sid, {
      session_id: sid,
      buffer_index: 0,
      item: {
        type: "reasoning",
        id: "rs_1",
        summary: [],
        content: [{ type: "reasoning_text", text: "think" }],
      },
    });
    slice = useMessageStore.getState().bySession.get(sid)!;
    // After first seal: committed prefix sorted; remaining live message follows.
    expect(slice.messages.map((m) => m.item.type)).toEqual(["reasoning", "message"]);
    expect(rowBufferIndex(slice.messages[0])).toBe(0);
    expect(slice.messages[1].id).toBe(liveItemRowId("msg_out"));

    useMessageStore.getState().onBufferItem(sid, {
      session_id: sid,
      buffer_index: 1,
      item: assistantMsg("msg_out", "answer"),
    });
    slice = useMessageStore.getState().bySession.get(sid)!;
    expect(slice.messages.map((m) => m.item.type)).toEqual(["reasoning", "message"]);
    expect(slice.messages.map((m) => rowBufferIndex(m))).toEqual([0, 1]);
  });

  it("does not overwrite function_call with function_call_output sharing call_id", () => {
    const sid = "s-tool";
    const callId = "call_1";

    useMessageStore.getState().applyStreamEvent(sid, "t1", 1, {
      type: "response.function_call_arguments.delta",
      sequence_number: 1,
      item_id: callId,
      output_index: 0,
      delta: "{}",
    });
    let slice = useMessageStore.getState().bySession.get(sid)!;
    expect(slice.messages).toHaveLength(1);
    expect(slice.messages[0].item.type).toBe("function_call");

    useMessageStore.getState().onBufferItem(sid, {
      session_id: sid,
      buffer_index: 0,
      item: functionCall(callId, "bash"),
    });
    slice = useMessageStore.getState().bySession.get(sid)!;
    expect(slice.messages).toHaveLength(1);
    expect(slice.messages[0].item.type).toBe("function_call");

    useMessageStore.getState().onBufferItem(sid, {
      session_id: sid,
      buffer_index: 1,
      item: functionCallOutput(callId, "ok"),
    });
    slice = useMessageStore.getState().bySession.get(sid)!;
    expect(slice.messages).toHaveLength(2);
    expect(slice.messages.map((m) => m.item.type)).toEqual([
      "function_call",
      "function_call_output",
    ]);
    expect(slice.shapeError).toBeNull();
  });

  it("invalidates in_progress function calls on a stream failure event (FE-06)", () => {
    const sid = "s-fail";
    // Two live tool calls in flight.
    useMessageStore.getState().applyStreamEvent(sid, "t1", 1, {
      type: "response.output_item.added",
      sequence_number: 1,
      output_index: 0,
      item: {
        type: "function_call",
        id: "fc_1",
        call_id: "call_1",
        name: "bash",
        arguments: "",
        status: "in_progress",
      },
    });
    useMessageStore.getState().applyStreamEvent(sid, "t1", 1, {
      type: "response.output_item.added",
      sequence_number: 2,
      output_index: 1,
      item: {
        type: "function_call",
        id: "fc_2",
        call_id: "call_2",
        name: "read",
        arguments: "",
        status: "in_progress",
      },
    });
    let slice = useMessageStore.getState().bySession.get(sid)!;
    expect(slice.messages).toHaveLength(2);
    expect(
      slice.messages.every(
        (m) => isFunctionCall(m.item) && m.item.status === "in_progress",
      ),
    ).toBe(true);

    // A turn-level failure must invalidate both, not leave them in_progress.
    useMessageStore.getState().applyStreamEvent(sid, "t1", 1, {
      type: "response.failed",
      response: {},
    });

    slice = useMessageStore.getState().bySession.get(sid)!;
    expect(
      slice.messages.every(
        (m) => isFunctionCall(m.item) && m.item.status === "failed",
      ),
    ).toBe(true);
    expect(slice.messages.every((m) => m.streaming === false)).toBe(true);
  });

  it("keeps live text through response.incomplete then seals on buffer/item", () => {
    const sid = "s-incomplete-seal";
    useMessageStore.getState().applyStreamEvent(sid, "t1", 1, {
      type: "response.output_text.delta",
      sequence_number: 1,
      item_id: "msg_inc",
      output_index: 0,
      content_index: 0,
      delta: "stopped mid",
    });
    useMessageStore.getState().applyStreamEvent(sid, "t1", 1, {
      type: "response.incomplete",
      response: {},
    });

    let slice = useMessageStore.getState().bySession.get(sid)!;
    expect(slice.messages).toHaveLength(1);
    expect(slice.messages[0].streaming).toBe(true);
    expect(itemPlainText(slice.messages[0].item)).toBe("stopped mid");

    useMessageStore.getState().onBufferItem(sid, {
      session_id: sid,
      buffer_index: 0,
      item: {
        type: "message",
        role: "assistant",
        id: "msg_inc",
        status: "incomplete",
        content: [{ type: "output_text", text: "stopped mid", annotations: [] }],
      },
    });
    slice = useMessageStore.getState().bySession.get(sid)!;
    expect(slice.messages).toHaveLength(1);
    expect(slice.messages[0].streaming).toBe(false);
    expect(slice.messages[0].item).toMatchObject({
      type: "message",
      id: "msg_inc",
      status: "incomplete",
    });
    expect(itemPlainText(slice.messages[0].item)).toBe("stopped mid");
  });

  it("dedups an optimistic user row when buffer/load returns the same message (FE-05)", () => {
    const sid = "s-overlap";
    // Optimistic user row (id `user-*`, no buffer id) added on send.
    useMessageStore.getState().pushUserMessage(sid, {
      id: "user-1-123",
      item: {
        type: "message",
        role: "user",
        content: [{ type: "input_text", text: "hello agent" }],
      },
    });
    // buffer/load arrives with the same message now committed at index 0.
    loadBuffer(sid, {
      session_id: sid,
      start: 0,
      end: 1,
      items: [
        {
          type: "message",
          role: "user",
          content: [{ type: "input_text", text: "hello agent" }],
        },
      ],
      user_detail_before: 0,
    });

    const slice = useMessageStore.getState().bySession.get(sid)!;
    expect(slice.messages).toHaveLength(1);
    expect(slice.messages[0].id).toBe("ord-0");
    expect(rowBufferIndex(slice.messages[0])).toBe(0);
    expect(slice.messages.some((m) => m.id.startsWith("user-"))).toBe(false);
  });

  it("keeps an optimistic user row when buffer/load has no matching message (FE-05)", () => {
    const sid = "s-no-overlap";
    useMessageStore.getState().pushUserMessage(sid, {
      id: "user-2-456",
      item: {
        type: "message",
        role: "user",
        content: [{ type: "input_text", text: "pending send" }],
      },
    });
    // History load that does NOT include the pending message.
    loadBuffer(sid, {
      session_id: sid,
      start: 0,
      end: 1,
      items: [
        {
          type: "message",
          role: "user",
          content: [{ type: "input_text", text: "older message" }],
        },
      ],
      user_detail_before: 0,
    });

    const slice = useMessageStore.getState().bySession.get(sid)!;
    expect(slice.messages.some((m) => m.id.startsWith("user-"))).toBe(true);
  });

  it("applies early name from output_item.added via nested item id", () => {
    const sid = "s-early-name";
    useMessageStore.getState().applyStreamEvent(sid, "t1", 1, {
      type: "response.output_item.added",
      sequence_number: 1,
      output_index: 0,
      item: {
        type: "function_call",
        id: "fc_1",
        call_id: "call_1",
        name: "read",
        arguments: "",
        status: "in_progress",
      },
    });
    let slice = useMessageStore.getState().bySession.get(sid)!;
    expect(slice.messages).toHaveLength(1);
    expect(slice.messages[0].streaming).toBe(true);
    expect(slice.messages[0].item).toMatchObject({
      type: "function_call",
      id: "fc_1",
      call_id: "call_1",
      name: "read",
    });

    useMessageStore.getState().applyStreamEvent(sid, "t1", 1, {
      type: "response.function_call_arguments.delta",
      sequence_number: 2,
      item_id: "fc_1",
      output_index: 0,
      delta: '{"path":"x"}',
    });
    slice = useMessageStore.getState().bySession.get(sid)!;
    expect(slice.messages).toHaveLength(1);
    expect(slice.messages[0].item).toMatchObject({
      name: "read",
      arguments: '{"path":"x"}',
    });
  });

  it("buffer/item type mismatch on same buffer id is fail-closed", () => {
    const sid = "s2";
    // Pre-seal a message at buffer index 0, then a conflicting type arrives on that slot id.
    useMessageStore.setState({
      bySession: new Map([
        [
          sid,
          {
            messages: [
              {
                id: liveItemRowId("x1"),
                bufferIndex: 0,
                item: assistantMsg("x1", "hi"),
                streaming: true,
              },
            ],
            bufferViewStart: 0,
            bufferViewEnd: 0,
            committedBufferEnd: 0,
            userDetailBefore: 0,
            loadingHistory: false,
            shapeError: null,
            subagentBindings: {},
            blockLogGrowth: false,
            turnEndNotice: null,
          },
        ],
      ]),
    });

    useMessageStore.getState().onBufferItem(sid, {
      session_id: sid,
      buffer_index: 0,
      item: {
        type: "reasoning",
        id: "x1",
        summary: [],
      },
    });
    const slice = useMessageStore.getState().bySession.get(sid)!;
    expect(slice.shapeError).toMatch(/type=/);
    expect(slice.messages).toHaveLength(1);
    expect(slice.messages[0].item.type).toBe("reasoning");
  });

  it("buffer/item uses Item id to seal; a different id at the same index is not a seal", () => {
    const sid = "s-id-not-index";
    markTurnRunning(sid);
    useMessageStore.getState().onBufferItem(sid, {
      session_id: sid,
      buffer_index: 0,
      item: assistantMsg(
        "msg_02178717097751000000000000000000000ffffac1578d682a35b",
        "old slot",
      ),
      kind: "detail",
    });
    useMessageStore.getState().applyStreamEvent(sid, "t1", 1, {
      type: "response.output_item.added",
      sequence_number: 1,
      output_index: 0,
      item: {
        type: "function_call",
        id: "call_kcxgeywsuqtgdg1w3fmx8zzt",
        call_id: "call_kcxgeywsuqtgdg1w3fmx8zzt",
        name: "bash",
        arguments: "{}",
        status: "in_progress",
      },
    });
    useMessageStore.getState().onBufferItem(sid, {
      session_id: sid,
      buffer_index: 0,
      item: functionCall("call_kcxgeywsuqtgdg1w3fmx8zzt", "bash"),
      kind: "detail",
    });
    let slice = useMessageStore.getState().bySession.get(sid)!;
    expect(slice.shapeError).toBeNull();
    expect(useToastStore.getState().toasts).toHaveLength(0);
    expect(
      slice.messages.some(
        (row) =>
          isFunctionCall(row.item) &&
          row.item.call_id === "call_kcxgeywsuqtgdg1w3fmx8zzt" &&
          rowBufferIndex(row) === 0,
      ),
    ).toBe(true);
    expect(
      slice.messages.some(
        (row) =>
          row.item.type === "message" &&
          itemAuthorityId(row.item) ===
            "msg_02178717097751000000000000000000000ffffac1578d682a35b" &&
          rowBufferIndex(row) == null,
      ),
    ).toBe(true);

    useMessageStore.getState().onBufferItem(sid, {
      session_id: sid,
      buffer_index: 1,
      item: assistantMsg(
        "msg_02178717097751000000000000000000000ffffac1578d682a35b",
        "old slot",
      ),
      kind: "detail",
    });
    slice = useMessageStore.getState().bySession.get(sid)!;
    expect(rowBufferIndex(slice.messages[1]!)).toBe(1);
    expect(itemAuthorityId(slice.messages[1]!.item)).toBe(
      "msg_02178717097751000000000000000000000ffffac1578d682a35b",
    );
    expect(slice.messages.every((row) => rowBufferIndex(row) != null)).toBe(true);
  });

  it("a single hole after compact does not drop the rest of the turn", () => {
    const sid = "s-hole-highwater";
    loadBuffer(sid, {
      session_id: sid,
      start: 0,
      end: 2,
      items: [assistantMsg("a0", "kept"), assistantMsg("a1", "kept2")],
      kinds: ["detail", "detail"],
      user_detail_before: 0,
    });
    // Checkpoint at 2 never arrives (hole). Next turn starts at 3.
    useMessageStore.getState().onBufferItem(sid, {
      session_id: sid,
      buffer_index: 3,
      item: userMsg("现在帮我看下本地nightly的构建流程"),
      kind: "detail",
    });
    useMessageStore.getState().onBufferItem(sid, {
      session_id: sid,
      buffer_index: 4,
      item: assistantMsg("msg_after", "still here"),
      kind: "detail",
    });
    const slice = useMessageStore.getState().bySession.get(sid)!;
    expect(slice.shapeError).toBeNull();
    expect(slice.committedBufferEnd).toBeGreaterThanOrEqual(5);
    expect(
      slice.messages.some((row) => itemPlainText(row.item) === "现在帮我看下本地nightly的构建流程"),
    ).toBe(true);
    expect(
      slice.messages.some((row) => itemPlainText(row.item) === "still here"),
    ).toBe(true);
  });

  it("records subagent bindings from live bind and buffer/load", () => {
    const sid = "s-bind";
    useMessageStore.getState().onSubagentBound(sid, {
      session_id: sid,
      call_id: "call_a",
      child_session_id: "child_a",
    });
    expect(useMessageStore.getState().bySession.get(sid)!.subagentBindings).toEqual({
      call_a: "child_a",
    });

    useMessageStore.getState().onBufferItem(sid, {
      session_id: sid,
      buffer_index: 0,
      item: functionCall("call_b", "subagent_launch"),
      child_session_id: "child_b",
    });
    expect(useMessageStore.getState().bySession.get(sid)!.subagentBindings).toEqual({
      call_a: "child_a",
      call_b: "child_b",
    });

    loadBuffer(sid, {
      session_id: sid,
      start: 0,
      end: 0,
      items: [],
      user_detail_before: 0,
      subagent_bindings: { call_c: "child_c" },
    });
    expect(useMessageStore.getState().bySession.get(sid)!.subagentBindings).toEqual({
      call_a: "child_a",
      call_b: "child_b",
      call_c: "child_c",
    });
  });

  it("buffer/load sets userDetailBefore and does not stamp per-row anchors", () => {
    const sid = "s-anchor";
    const user = (text: string): Item => ({
      type: "message",
      role: "user",
      content: [{ type: "input_text", text }],
    });
    const assistant = (id: string, text: string): Item =>
      assistantMsg(id, text);

    loadBuffer(sid, {
      session_id: sid,
      start: 10,
      end: 14,
      user_detail_before: 3,
      items: [
        user("u3"),
        assistant("a3", "ok"),
        user("u4"),
        assistant("a4", "ok"),
      ],
    });
    let slice = useMessageStore.getState().bySession.get(sid)!;
    expect(slice.userDetailBefore).toBe(3);
    expect(slice.bufferViewStart).toBe(10);
    expect(slice.messages).toHaveLength(4);
    for (const row of slice.messages) {
      expect(row).not.toHaveProperty("userAnchorK");
    }

    // Extending backward refreshes the baseline; suffix reload must not.
    loadBuffer(sid, {
      session_id: sid,
      start: 4,
      end: 10,
      user_detail_before: 1,
      items: [user("u1"), assistant("a1", "x"), user("u2"), assistant("a2", "y"), user("u2b"), assistant("a2b", "z")],
    });
    slice = useMessageStore.getState().bySession.get(sid)!;
    expect(slice.userDetailBefore).toBe(1);
    expect(slice.bufferViewStart).toBe(4);

    loadBuffer(sid, {
      session_id: sid,
      start: 12,
      end: 14,
      user_detail_before: 99,
      items: [user("u4"), assistant("a4", "ok")],
    });
    slice = useMessageStore.getState().bySession.get(sid)!;
    expect(slice.userDetailBefore).toBe(1);
    expect(slice.bufferViewStart).toBe(4);
  });

  it("REV-11: wire kinds let deriveUserAnchorK exclude compact checkpoints in production", () => {
    const sid = "s-rev11";
    const user = (text: string): Item => ({
      type: "message",
      role: "user",
      content: [{ type: "input_text", text }],
    });
    // u0,u1 were compacted into the summary checkpoint; u2,u3 remain.
    const checkpoint = (text: string): Item => ({
      type: "message",
      role: "user",
      content: [{ type: "input_text", text }],
    });

    loadBuffer(sid, {
      session_id: sid,
      start: 0,
      end: 4,
      user_detail_before: 0,
      // Production-shaped: the backend tags each row with its DB kind. The
      // compact checkpoint is a user-role row but must NOT count as an anchor.
      kinds: ["compact_checkpoint", "detail", "detail", "detail"],
      items: [checkpoint("summary"), user("u2"), assistantMsg("a2", "ok"), user("u3")],
    });

    const slice = useMessageStore.getState().bySession.get(sid)!;
    const checkpointRow = slice.messages.find(
      (m) => isUserMessage(m.item) && itemPlainText(m.item) === "summary",
    )!;
    expect(checkpointRow.kind).toBe("compact_checkpoint");

    // Backend-aligned k for u3 (the 2nd visible detail user): 0-based ordinal 1.
    const u3Idx = slice.messages.findIndex((m) => isUserMessage(m.item) && itemPlainText(m.item) === "u3");
    const k = deriveUserAnchorK(slice.messages, u3Idx, 0);
    expect(k).toBe(1); // == backend entry_user_detail_count semantic (u2,u3 only)
  });

  it("idle buffer/load drops unsealed live and shows committed authority", () => {
    const sid = "s-reconnect-seal";
    useMessageStore.getState().applyStreamEvent(sid, "t1", 1, {
      type: "response.output_text.delta",
      sequence_number: 1,
      item_id: "msg_live",
      output_index: 0,
      content_index: 0,
      delta: "hello",
    });
    const before = useMessageStore.getState().bySession.get(sid)!;
    expect(before.messages).toHaveLength(1);
    expect(before.messages[0].id).toBe(liveItemRowId("msg_live"));

    loadBuffer(sid, {
      session_id: sid,
      start: 0,
      end: 1,
      items: [assistantMsg("msg_live", "hello")],
      user_detail_before: 0,
    });

    const slice = useMessageStore.getState().bySession.get(sid)!;
    expect(slice.messages).toHaveLength(1);
    expect(slice.messages[0].id).toBe(liveItemRowId("msg_live"));
    expect(rowBufferIndex(slice.messages[0])).toBe(0);
    expect(slice.messages[0].streaming).toBe(false);
    expect(itemPlainText(slice.messages[0].item)).toBe("hello");
  });

  it("buffer/load replaces live content when it diverges from authority", () => {
    const sid = "s-reconnect-diverge";
    useMessageStore.getState().applyStreamEvent(sid, "t1", 1, {
      type: "response.output_text.delta",
      sequence_number: 1,
      item_id: "msg_div",
      output_index: 0,
      content_index: 0,
      delta: "partial",
    });
    loadBuffer(sid, {
      session_id: sid,
      start: 0,
      end: 1,
      items: [assistantMsg("msg_div", "partial and done")],
      user_detail_before: 0,
    });
    const slice = useMessageStore.getState().bySession.get(sid)!;
    expect(slice.messages).toHaveLength(1);
    expect(itemPlainText(slice.messages[0].item)).toBe("partial and done");
    expect(slice.messages[0].id).toBe(liveItemRowId("msg_div"));
    expect(rowBufferIndex(slice.messages[0])).toBe(0);
  });

  it("buffer/load keeps a live row that is not in the committed window", () => {
    const sid = "s-reconnect-keep-live";
    markTurnRunning(sid);
    useMessageStore.getState().pushUserMessage(sid, {
      id: "ord-0",
      bufferIndex: 0,
      item: {
        type: "message",
        role: "user",
        content: [{ type: "input_text", text: "older" }],
      },
    });
    useMessageStore.getState().applyStreamEvent(sid, "t1", 1, {
      type: "response.output_text.delta",
      sequence_number: 1,
      item_id: "msg_new",
      output_index: 0,
      content_index: 0,
      delta: "still streaming",
    });
    loadBuffer(sid, {
      session_id: sid,
      start: 0,
      end: 1,
      items: [
        {
          type: "message",
          role: "user",
          content: [{ type: "input_text", text: "older" }],
        },
      ],
      user_detail_before: 0,
    });
    const slice = useMessageStore.getState().bySession.get(sid)!;
    expect(slice.messages).toHaveLength(2);
    expect(slice.messages[1].id).toBe(liveItemRowId("msg_new"));
    expect(itemPlainText(slice.messages[1].item)).toBe("still streaming");
  });

  it("buffer/reverted drops the tail and unindexed live/optimistic rows", () => {
    const sid = "s-revert-tail";
    loadBuffer(sid, {
      session_id: sid,
      start: 0,
      end: 3,
      items: [
        {
          type: "message",
          role: "user",
          content: [{ type: "input_text", text: "keep" }],
        },
        assistantMsg("a0", "keep-reply"),
        {
          type: "message",
          role: "user",
          content: [{ type: "input_text", text: "drop-me" }],
        },
      ],
      user_detail_before: 0,
    });
    useMessageStore.getState().pushUserMessage(sid, {
      id: "user-orphan-1",
      item: {
        type: "message",
        role: "user",
        content: [{ type: "input_text", text: "optimistic leftover" }],
      },
    });
    useTurnStore.getState().onTurnStarted({
      session_id: sid,
      turn_id: "t-live",
      input: "",
      step_max: 1,
    });
    useMessageStore.getState().applyStreamEvent(sid, "t-live", 1, {
      type: "response.output_text.delta",
      sequence_number: 1,
      item_id: "msg_live_drop",
      output_index: 0,
      content_index: 0,
      delta: "should vanish",
    });

    useTurnStore.getState().onTranscriptReverted(sid);
    useMessageStore.getState().onBufferReverted(sid, {
      session_id: sid,
      committed_end: 2,
    });

    const slice = useMessageStore.getState().bySession.get(sid)!;
    expect(slice.messages.map((m) => itemPlainText(m.item))).toEqual([
      "keep",
      "keep-reply",
    ]);
    expect(slice.committedBufferEnd).toBe(2);
    expect(
      slice.messages.some((m) => m.id.startsWith("live-") && rowBufferIndex(m) == null),
    ).toBe(false);
    expect(slice.messages.some((m) => m.id.startsWith("user-"))).toBe(false);

    useMessageStore.getState().applyStreamEvent(sid, "t-live", 1, {
      type: "response.output_text.delta",
      sequence_number: 2,
      item_id: "msg_live_drop",
      output_index: 0,
      content_index: 0,
      delta: " after revert",
    });
    expect(useMessageStore.getState().bySession.get(sid)!.messages).toHaveLength(2);

    useMessageStore.getState().onBufferItem(sid, {
      session_id: sid,
      buffer_index: 3,
      item: assistantMsg("late", "replayed tail"),
    });
    expect(useMessageStore.getState().bySession.get(sid)!.messages).toHaveLength(2);

    useMessageStore.getState().onBufferItem(sid, {
      session_id: sid,
      buffer_index: 2,
      item: assistantMsg("late-eq", "first deleted row"),
    });
    expect(useMessageStore.getState().bySession.get(sid)!.messages).toHaveLength(2);
    expect(useMessageStore.getState().bySession.get(sid)!.committedBufferEnd).toBe(2);

    loadBuffer(sid, {
      session_id: sid,
      start: 0,
      end: 4,
      items: [
        {
          type: "message",
          role: "user",
          content: [{ type: "input_text", text: "keep" }],
        },
        assistantMsg("a0", "keep-reply"),
        {
          type: "message",
          role: "user",
          content: [{ type: "input_text", text: "drop-me" }],
        },
        assistantMsg("late-load", "stale tail"),
      ],
      user_detail_before: 0,
    });
    expect(useMessageStore.getState().bySession.get(sid)!.messages).toHaveLength(2);
    expect(useMessageStore.getState().bySession.get(sid)!.committedBufferEnd).toBe(2);

    useTurnStore.getState().onTurnStarted({
      session_id: sid,
      turn_id: "t-next",
      input: "",
      step_max: 1,
    });
    useMessageStore.getState().onBufferItem(sid, {
      session_id: sid,
      buffer_index: 2,
      item: assistantMsg("next", "new turn first"),
    });
    const after = useMessageStore.getState().bySession.get(sid)!;
    expect(after.committedBufferEnd).toBe(3);
    expect(itemPlainText(after.messages[2].item)).toBe("new turn first");
  });

  it("finalizeTurn drops unsealed live rows and keeps sealed plus optimistic user", () => {
    const sid = "s-finalize-drop-live";
    markTurnRunning(sid);
    useMessageStore.getState().onBufferItem(sid, {
      session_id: sid,
      buffer_index: 0,
      item: assistantMsg("a0", "sealed"),
    });
    useMessageStore.getState().pushUserMessage(sid, {
      id: "user-next",
      item: {
        type: "message",
        role: "user",
        content: [{ type: "input_text", text: "continue" }],
      },
    });
    useMessageStore.getState().applyStreamEvent(sid, "t1", 1, {
      type: "response.reasoning_text.delta",
      sequence_number: 1,
      item_id: "rs_half",
      output_index: 0,
      content_index: 0,
      delta: "half thought",
    });

    useMessageStore.getState().finalizeTurn(sid, "t1");

    const slice = useMessageStore.getState().bySession.get(sid)!;
    expect(slice.messages.map((m) => m.id)).toEqual([
      liveItemRowId("a0"),
      "user-next",
    ]);
    expect(slice.messages.every((m) => m.streaming !== true)).toBe(true);
  });

  it("buffer/load while idle drops unclaimed live overlay", () => {
    const sid = "s-idle-drop-live";
    useMessageStore.getState().applyStreamEvent(sid, "t1", 1, {
      type: "response.output_text.delta",
      sequence_number: 1,
      item_id: "ghost",
      output_index: 0,
      content_index: 0,
      delta: "ghost text",
    });
    loadBuffer(sid, {
      session_id: sid,
      start: 0,
      end: 1,
      items: [
        {
          type: "message",
          role: "user",
          content: [{ type: "input_text", text: "hi" }],
        },
      ],
      user_detail_before: 0,
    });
    const slice = useMessageStore.getState().bySession.get(sid)!;
    expect(slice.messages).toHaveLength(1);
    expect(itemPlainText(slice.messages[0].item)).toBe("hi");
    expect(slice.messages.some((m) => m.id.startsWith("live-"))).toBe(false);
  });

  it("compact then next turn seals optimistic user before assistant", () => {
    const sid = "s-compact-next-turn";
    loadBuffer(sid, {
      session_id: sid,
      start: 0,
      end: 1,
      items: [assistantMsg("a0", "kept")],
      kinds: ["detail"],
      user_detail_before: 0,
    });
    useMessageStore.getState().onBufferItem(sid, {
      session_id: sid,
      buffer_index: 1,
      item: userMsg("rolled-up"),
      kind: "compact_checkpoint",
    });

    markTurnRunning(sid);
    useMessageStore.getState().pushUserMessage(sid, {
      id: "user-next-1",
      item: userMsg("continue after compact"),
    });
    useMessageStore.getState().onBufferItem(sid, {
      session_id: sid,
      buffer_index: 2,
      item: userMsg("continue after compact"),
      kind: "detail",
    });
    useMessageStore.getState().onBufferItem(sid, {
      session_id: sid,
      buffer_index: 3,
      item: assistantMsg("a1", "reply"),
      kind: "detail",
    });

    const slice = useMessageStore.getState().bySession.get(sid)!;
    expect(slice.messages.map((row) => itemPlainText(row.item))).toEqual([
      "kept",
      "rolled-up",
      "continue after compact",
      "reply",
    ]);
    expect(slice.messages[1]?.kind).toBe("compact_checkpoint");
    expect(slice.messages.some((row) => row.id.startsWith("user-"))).toBe(false);
  });

  it("buffer/item checkpoint does not consume a different optimistic user", () => {
    const sid = "s-cp-no-steal";
    useMessageStore.getState().pushUserMessage(sid, {
      id: "user-pending",
      item: userMsg("hello after compact"),
    });
    useMessageStore.getState().onBufferItem(sid, {
      session_id: sid,
      buffer_index: 0,
      item: userMsg("rolled-up summary"),
      kind: "compact_checkpoint",
    });
    const slice = useMessageStore.getState().bySession.get(sid)!;
    expect(slice.messages[0]?.kind).toBe("compact_checkpoint");
    expect(itemPlainText(slice.messages[0]!.item)).toBe("rolled-up summary");
    expect(slice.messages.some((row) => row.id === "user-pending")).toBe(true);
    expect(itemPlainText(slice.messages[1]!.item)).toBe("hello after compact");
  });

  it("buffer/item overlays an occupied index instead of skipping", () => {
    const sid = "s-overlay-user";
    useMessageStore.getState().onBufferItem(sid, {
      session_id: sid,
      buffer_index: 0,
      item: userMsg("stale"),
      kind: "detail",
    });
    useMessageStore.getState().pushUserMessage(sid, {
      id: "user-fresh",
      item: userMsg("fresh"),
    });
    useMessageStore.getState().onBufferItem(sid, {
      session_id: sid,
      buffer_index: 0,
      item: userMsg("fresh"),
      kind: "detail",
    });
    const slice = useMessageStore.getState().bySession.get(sid)!;
    expect(slice.messages).toHaveLength(1);
    expect(slice.messages[0]?.id).toBe("ord-0");
    expect(rowBufferIndex(slice.messages[0]!)).toBe(0);
    expect(itemPlainText(slice.messages[0]!.item)).toBe("fresh");
  });

  it("buffer/load does not text-dedup optimistic user against a checkpoint", () => {
    const sid = "s-fe05-cp";
    useMessageStore.getState().pushUserMessage(sid, {
      id: "user-same-text",
      item: userMsg("summary"),
    });
    loadBuffer(sid, {
      session_id: sid,
      start: 0,
      end: 1,
      items: [userMsg("summary")],
      kinds: ["compact_checkpoint"],
      user_detail_before: 0,
    });
    const slice = useMessageStore.getState().bySession.get(sid)!;
    expect(slice.messages).toHaveLength(2);
    expect(slice.messages[0]?.kind).toBe("compact_checkpoint");
    expect(slice.messages.some((row) => row.id === "user-same-text")).toBe(true);
  });

  it("dedups an optimistic kill reminder when buffer/item seals the same text", () => {
    const sid = "s-kill-reminder";
    const reminder =
      "<system-reminder>\nThe user stopped background bash bg_a (Kill).\n</system-reminder>";
    useMessageStore.getState().pushUserMessage(sid, {
      id: "user-kill-1",
      item: userMsg(reminder),
    });
    useMessageStore.getState().onBufferItem(sid, {
      session_id: sid,
      buffer_index: 0,
      item: userMsg(reminder),
      kind: "detail",
    });
    const slice = useMessageStore.getState().bySession.get(sid)!;
    expect(slice.messages).toHaveLength(1);
    expect(slice.messages[0]?.id).toBe("ord-0");
    expect(rowBufferIndex(slice.messages[0]!)).toBe(0);
    expect(slice.messages.some((row) => row.id.startsWith("user-"))).toBe(false);
  });

  it("buffer/item still seals a matching live function_call", () => {
    const sid = "s-item-seal-fc";
    markTurnRunning(sid);
    useMessageStore.getState().applyStreamEvent(sid, "t1", 1, {
      type: "response.output_item.added",
      sequence_number: 1,
      output_index: 0,
      item: {
        type: "function_call",
        id: "call_koxta",
        call_id: "call_koxta",
        name: "bash",
        arguments: "{}",
        status: "in_progress",
      },
    });
    useMessageStore.getState().onBufferItem(sid, {
      session_id: sid,
      buffer_index: 0,
      item: functionCall("call_koxta", "bash"),
      kind: "detail",
    });
    const slice = useMessageStore.getState().bySession.get(sid)!;
    expect(slice.shapeError).toBeNull();
    expect(useToastStore.getState().toasts).toHaveLength(0);
    expect(slice.messages).toHaveLength(1);
    expect(slice.messages[0]?.id).toBe(liveItemRowId("call_koxta"));
    expect(rowBufferIndex(slice.messages[0]!)).toBe(0);
    expect(isFunctionCall(slice.messages[0]!.item)).toBe(true);
  });

  it("buffer/item does not seal a live row whose item id moved off the row key", () => {
    const sid = "s-seal-stale-row-id";
    markTurnRunning(sid);
    useMessageStore.getState().applyStreamEvent(sid, "t1", 1, {
      type: "response.output_item.added",
      sequence_number: 1,
      output_index: 0,
      item: {
        type: "function_call",
        id: "call_koxta",
        call_id: "call_koxta",
        name: "bash",
        arguments: "{}",
        status: "in_progress",
      },
    });
    useMessageStore.getState().applyStreamEvent(sid, "t1", 1, {
      type: "response.output_item.added",
      sequence_number: 2,
      output_index: 1,
      item: {
        type: "function_call",
        id: "call_nqvubr",
        call_id: "call_nqvubr",
        name: "bash",
        arguments: "{}",
        status: "in_progress",
      },
    });
    // Simulate a live shell keyed by the first call_id whose Item mutated.
    const mid = useMessageStore.getState().bySession.get(sid)!;
    const live = mid.messages.find((row) => row.id === liveItemRowId("call_koxta"));
    if (live) {
      useMessageStore.setState({
        bySession: new Map([
          [
            sid,
            {
              ...mid,
              messages: mid.messages.map((row) =>
                row.id === liveItemRowId("call_koxta")
                  ? { ...row, item: functionCall("call_nqvubr", "bash") }
                  : row,
              ),
            },
          ],
        ]),
      });
    }
    useMessageStore.getState().onBufferItem(sid, {
      session_id: sid,
      buffer_index: 0,
      item: functionCall("call_koxta", "bash"),
      kind: "detail",
    });
    const slice = useMessageStore.getState().bySession.get(sid)!;
    expect(slice.shapeError).toBeNull();
    expect(useToastStore.getState().toasts).toHaveLength(0);
    expect(
      slice.messages.some(
        (row) =>
          isFunctionCall(row.item) &&
          row.item.call_id === "call_nqvubr" &&
          rowBufferIndex(row) == null,
      ),
    ).toBe(true);
    expect(
      slice.messages.some(
        (row) =>
          isFunctionCall(row.item) &&
          row.item.call_id === "call_koxta" &&
          rowBufferIndex(row) === 0,
      ),
    ).toBe(true);
  });

  it("late compact last-40 load does not steal the next turn live function_call", () => {
    const sid = "s-compact-late-load";
    loadBuffer(sid, {
      session_id: sid,
      start: 0,
      end: 2,
      items: [assistantMsg("rs_kept", "kept"), functionCall("call_hist", "read")],
      kinds: ["detail", "detail"],
      user_detail_before: 0,
    });
    useMessageStore.getState().onBufferItem(sid, {
      session_id: sid,
      buffer_index: 2,
      item: userMsg("rolled-up"),
      kind: "compact_checkpoint",
    });
    markTurnRunning(sid);
    useMessageStore.getState().pushUserMessage(sid, {
      id: "user-nightly",
      item: userMsg("现在帮我看下本地nightly的构建流程"),
    });
    useMessageStore.getState().onBufferItem(sid, {
      session_id: sid,
      buffer_index: 3,
      item: userMsg("现在帮我看下本地nightly的构建流程"),
      kind: "detail",
    });
    useMessageStore.getState().applyStreamEvent(sid, "t1", 1, {
      type: "response.output_item.added",
      sequence_number: 1,
      output_index: 0,
      item: {
        type: "function_call",
        id: "call_nqvubr",
        call_id: "call_nqvubr",
        name: "bash",
        arguments: "{}",
        status: "in_progress",
      },
    });

    loadBuffer(sid, {
      session_id: sid,
      start: 0,
      end: 3,
      items: [
        assistantMsg("rs_kept", "kept"),
        functionCall("call_hist", "read"),
        userMsg("rolled-up"),
      ],
      kinds: ["detail", "detail", "compact_checkpoint"],
      user_detail_before: 0,
    });

    const slice = useMessageStore.getState().bySession.get(sid)!;
    expect(slice.shapeError).toBeNull();
    expect(useToastStore.getState().toasts).toHaveLength(0);
    expect(itemPlainText(slice.messages[2]!.item)).toBe("rolled-up");
    expect(slice.messages[2]?.kind).toBe("compact_checkpoint");
    expect(rowBufferIndex(slice.messages[2]!)).toBe(2);
    expect(
      slice.messages.some((row) => itemPlainText(row.item) === "现在帮我看下本地nightly的构建流程"),
    ).toBe(true);
    expect(
      slice.messages.some(
        (row) =>
          isFunctionCall(row.item) &&
          row.item.call_id === "call_nqvubr" &&
          rowBufferIndex(row) == null,
      ),
    ).toBe(true);
    expect(rowBufferIndex(slice.messages[0]!)).toBe(0);
    expect(rowBufferIndex(slice.messages[1]!)).toBe(1);
  });

  it("buffer/load does not claim a live function_call with a different call_id", () => {
    const sid = "s-load-no-steal-fc";
    markTurnRunning(sid);
    useMessageStore.getState().applyStreamEvent(sid, "t1", 1, {
      type: "response.output_item.added",
      sequence_number: 1,
      output_index: 0,
      item: {
        type: "function_call",
        id: "call_nqvubr",
        call_id: "call_nqvubr",
        name: "bash",
        arguments: "{}",
        status: "in_progress",
      },
    });
    loadBuffer(sid, {
      session_id: sid,
      start: 0,
      end: 1,
      items: [functionCall("call_koxta", "bash")],
      kinds: ["detail"],
      user_detail_before: 0,
    });
    const slice = useMessageStore.getState().bySession.get(sid)!;
    expect(slice.shapeError).toBeNull();
    expect(
      useToastStore.getState().toasts.some((t) => t.message.includes("seal mismatch")),
    ).toBe(false);
    expect(
      slice.messages.some(
        (row) =>
          isFunctionCall(row.item) &&
          row.item.call_id === "call_nqvubr" &&
          rowBufferIndex(row) == null,
      ),
    ).toBe(true);
    expect(
      slice.messages.some(
        (row) =>
          isFunctionCall(row.item) &&
          row.item.call_id === "call_koxta" &&
          rowBufferIndex(row) === 0,
      ),
    ).toBe(true);
  });

  it("stamps bufferIndex from wire indices, not start+i", () => {
    const sid = "s-wire-indices";
    loadBuffer(sid, {
      session_id: sid,
      start: 0,
      end: 2,
      items: [assistantMsg("a0", "first"), assistantMsg("a1", "second")],
      kinds: ["detail", "detail"],
      indices: [10, 20],
      user_detail_before: 0,
    });
    const slice = useMessageStore.getState().bySession.get(sid)!;
    expect(slice.messages.map((row) => rowBufferIndex(row))).toEqual([10, 20]);
    expect(slice.messages.map((row) => row.id)).toEqual([
      liveItemRowId("a0"),
      liveItemRowId("a1"),
    ]);
  });

  it("rejects buffer/load that omits history indices", () => {
    const sid = "s-reject-indices";
    useMessageStore.getState().onBufferLoaded(sid, {
      session_id: sid,
      start: 0,
      end: 1,
      items: [assistantMsg("a0", "x")],
      user_detail_before: 0,
    } as BufferLoaded);
    const slice = useMessageStore.getState().bySession.get(sid)!;
    expect(slice.messages).toHaveLength(0);
    expect(slice.shapeError).toMatch(/indices/);
  });
});

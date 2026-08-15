/**
 * Low-churn business projection: stream upsert then buffer/item seal,
 * including seal mismatch → shapeError (fail-closed: no overwrite).
 */
import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  bufferItemId,
  deriveUserAnchorK,
  extractBufferIndex,
  isCompactCutRow,
  isFunctionCall,
  isMessageItem,
  isUserMessage,
  itemPlainText,
  liveItemRowId,
} from "../api/adapter";
import type { Item } from "../api/types";
import { useConnectionStore } from "./connectionStore";
import { applyCompactSplice, emptySlice, useMessageStore } from "./messageStore";
import { useToastStore } from "./toastStore";

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

describe("messageStore Item projection", () => {
  beforeEach(() => {
    useMessageStore.setState({ bySession: new Map() });
    useToastStore.setState({ toasts: [] });
  });

  it("buffer/compacted keeps existing rows and appends the checkpoint", async () => {
    const sid = "s-compact";
    useMessageStore.getState().onBufferItem(sid, {
      session_id: sid,
      buffer_index: 0,
      item: assistantMsg("old", "kept"),
    });
    useMessageStore.getState().applyStreamEvent(sid, "t1", 1, {
      type: "response.output_text.delta",
      sequence_number: 1,
      item_id: "msg_live",
      output_index: 0,
      content_index: 0,
      delta: "streaming",
    });
    const sendRpc = vi.fn(async () => ({
      session_id: sid,
      start: 1,
      end: 2,
      items: [assistantMsg("summary", "hidden summary")],
      kinds: ["compact_checkpoint"],
      user_detail_before: 0,
    }));
    useConnectionStore.setState({ sendRpc } as never);

    useMessageStore.getState().onBufferCompacted(sid, {
      session_id: sid,
      revision: 2,
      committed_end: 2,
    });
    await vi.waitFor(() => {
      expect(
        useMessageStore.getState().bySession.get(sid)?.messages.some(isCompactCutRow),
      ).toBe(true);
    });

    const slice = useMessageStore.getState().bySession.get(sid)!;
    expect(slice.messages[0]?.item).toEqual(assistantMsg("old", "kept"));
    expect(slice.messages[1]?.kind).toBe("compact_checkpoint");
    expect(slice.messages.some((row) => row.id === liveItemRowId("msg_live"))).toBe(
      true,
    );
    expect(sendRpc).toHaveBeenCalledWith("buffer/load", {
      start: 1,
      end: 2,
      session_id: sid,
    });
  });

  it("buffer/compacted replaces a prior checkpoint without dropping later details", async () => {
    const sid = "s-compact-2";
    useMessageStore.getState().onBufferLoaded(sid, {
      session_id: sid,
      start: 0,
      end: 3,
      items: [
        assistantMsg("a0", "first"),
        assistantMsg("sum1", "old summary"),
        assistantMsg("a1", "after"),
      ],
      kinds: ["detail", "compact_checkpoint", "detail"],
      user_detail_before: 0,
    });
    const sendRpc = vi.fn(async () => ({
      session_id: sid,
      start: 2,
      end: 3,
      items: [assistantMsg("sum2", "new summary")],
      kinds: ["compact_checkpoint"],
      user_detail_before: 0,
    }));
    useConnectionStore.setState({ sendRpc } as never);

    useMessageStore.getState().onBufferCompacted(sid, {
      session_id: sid,
      revision: 3,
      committed_end: 3,
    });
    await vi.waitFor(() => {
      const rows = useMessageStore.getState().bySession.get(sid)?.messages ?? [];
      expect(rows[2]?.kind).toBe("compact_checkpoint");
    });

    const slice = useMessageStore.getState().bySession.get(sid)!;
    expect(slice.messages.map((row) => row.item)).toEqual([
      assistantMsg("a0", "first"),
      assistantMsg("a1", "after"),
      assistantMsg("sum2", "new summary"),
    ]);
    expect(extractBufferIndex(slice.messages[1]!.id)).toBe(1);
    expect(sendRpc).toHaveBeenCalledWith("buffer/load", {
      start: 2,
      end: 3,
      session_id: sid,
    });
  });

  it("applyCompactSplice shifts the window when the old checkpoint is not loaded", () => {
    const sid = "s-compact-shift";
    const slice = {
      ...emptySlice(),
      bufferViewStart: 10,
      bufferViewEnd: 12,
      committedBufferEnd: 12,
      messages: [
        {
          id: bufferItemId(sid, 10),
          item: assistantMsg("a10", "kept"),
          kind: "detail",
        },
        {
          id: bufferItemId(sid, 11),
          item: assistantMsg("a11", "also"),
          kind: "detail",
        },
      ],
    };
    const next = applyCompactSplice(sid, slice, 12);
    expect(next.bufferViewStart).toBe(9);
    expect(extractBufferIndex(next.messages[0]!.id)).toBe(9);
    expect(extractBufferIndex(next.messages[1]!.id)).toBe(10);
    expect(next.messages.some(isCompactCutRow)).toBe(false);
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
    expect(extractBufferIndex(slice.messages[0].id)).toBe(0);
    expect(slice.messages[1].id).toBe(liveItemRowId("msg_out"));

    useMessageStore.getState().onBufferItem(sid, {
      session_id: sid,
      buffer_index: 1,
      item: assistantMsg("msg_out", "answer"),
    });
    slice = useMessageStore.getState().bySession.get(sid)!;
    expect(slice.messages.map((m) => m.item.type)).toEqual(["reasoning", "message"]);
    expect(slice.messages.map((m) => extractBufferIndex(m.id))).toEqual([0, 1]);
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
    useMessageStore.getState().onBufferLoaded(sid, {
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
    expect(slice.messages[0].id).toBe(bufferItemId(sid, 0));
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
    useMessageStore.getState().onBufferLoaded(sid, {
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
                id: bufferItemId(sid, 0),
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
    expect(slice.messages).toHaveLength(2);
    expect(slice.messages[0].item.type).toBe("message");
    expect(slice.messages[1].item.type).toBe("reasoning");
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

    useMessageStore.getState().onBufferLoaded(sid, {
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

    useMessageStore.getState().onBufferLoaded(sid, {
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
    useMessageStore.getState().onBufferLoaded(sid, {
      session_id: sid,
      start: 4,
      end: 10,
      user_detail_before: 1,
      items: [user("u1"), assistant("a1", "x"), user("u2"), assistant("a2", "y"), user("u2b"), assistant("a2b", "z")],
    });
    slice = useMessageStore.getState().bySession.get(sid)!;
    expect(slice.userDetailBefore).toBe(1);
    expect(slice.bufferViewStart).toBe(4);

    useMessageStore.getState().onBufferLoaded(sid, {
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

    useMessageStore.getState().onBufferLoaded(sid, {
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

  it("buffer/load seals a live assistant row instead of duplicating it", () => {
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
    const liveContent = isMessageItem(before.messages[0].item)
      ? before.messages[0].item.content
      : null;

    useMessageStore.getState().onBufferLoaded(sid, {
      session_id: sid,
      start: 0,
      end: 1,
      items: [assistantMsg("msg_live", "hello")],
      user_detail_before: 0,
    });

    const slice = useMessageStore.getState().bySession.get(sid)!;
    expect(slice.messages).toHaveLength(1);
    expect(slice.messages[0].id).toBe(bufferItemId(sid, 0));
    expect(slice.messages[0].streaming).toBe(false);
    expect(itemPlainText(slice.messages[0].item)).toBe("hello");
    expect(slice.messages.some((m) => m.id.startsWith("live-"))).toBe(false);
    expect(
      isMessageItem(slice.messages[0].item) && slice.messages[0].item.content,
    ).toBe(liveContent);
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
    useMessageStore.getState().onBufferLoaded(sid, {
      session_id: sid,
      start: 0,
      end: 1,
      items: [assistantMsg("msg_div", "partial and done")],
      user_detail_before: 0,
    });
    const slice = useMessageStore.getState().bySession.get(sid)!;
    expect(slice.messages).toHaveLength(1);
    expect(itemPlainText(slice.messages[0].item)).toBe("partial and done");
    expect(slice.messages[0].id).toBe(bufferItemId(sid, 0));
  });

  it("buffer/load keeps a live row that is not in the committed window", () => {
    const sid = "s-reconnect-keep-live";
    useMessageStore.getState().pushUserMessage(sid, {
      id: bufferItemId(sid, 0),
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
    useMessageStore.getState().onBufferLoaded(sid, {
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
});

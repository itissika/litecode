/**
 * Seq-keyed message store: load/item share one map; deltas only hit an existing seq.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { cleanup, render, screen } from "@testing-library/react";

import {
  deriveUserAnchorK,
  isCompactCutRow,
  itemPlainText,
} from "../api/adapter";
import type {
  BufferLoaded,
  Item,
  SessionSnapshot,
  WireBufferEvent,
} from "../api/types";
import { displayMessages, useMessageStore } from "./messageStore";
import { EMPTY_SLICE as EMPTY_TURN, useTurnStore } from "./turnStore";
import { useToastStore } from "./toastStore";
import { useConnectionStore } from "./connectionStore";
import { useSessionStore } from "./sessionStore";

function assistantMsg(
  id: string,
  text: string,
  status: "in_progress" | "completed" = "completed",
): Item {
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
  body: Item,
  state: "final" | "in_progress" = "final",
): WireBufferEvent {
  return {
    seq,
    kind:
      body.type === "message" && "role" in body && body.role === "user"
        ? "item/user"
        : "item/assistant",
    state,
    body,
  };
}

function load(
  sid: string,
  events: WireBufferEvent[],
  from = 0,
  to?: number,
  userDetailBefore?: number,
): void {
  const loaded: BufferLoaded = {
    session_id: sid,
    from_seq: from,
    to_seq:
      to ?? (events.length ? Math.max(...events.map((e) => e.seq)) + 1 : from),
    events,
    ...(userDetailBefore !== undefined
      ? { user_detail_before: userDetailBefore }
      : {}),
  };
  useMessageStore.getState().onBufferLoaded(sid, loaded);
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

describe("messageStore seq map", () => {
  beforeEach(() => {
    cleanup();
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
    expect(slice.messages.map((r) => itemPlainText(r.body as Item))).toEqual([
      "zero",
      "one",
      "two",
    ]);
    expect(slice.bySeq.size).toBe(3);
  });

  it("replace surface_op is a compact cut, not a user anchor", () => {
    const sid = "s-cut";
    load(sid, [
      ev(0, userMsg("ask")),
      ev(1, assistantMsg("a", "old")),
      {
        seq: 2,
        kind: "compacted",
        state: "final",
        body: { summary: "summary", from: 0, to: 2 },
      },
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
      events: [{ kind: "item/user", item: userMsg("x") } as never],
    });
    const slice = useMessageStore.getState().bySession.get(sid)!;
    expect(slice.messages).toHaveLength(0);
    expect(slice.shapeError).toMatch(/seq/);
  });

  it("a row the log still holds in flight is streaming without any payload status", () => {
    // The regression this guards: a reasoning item carries no `status`, so reading
    // lifecycle off the payload called the row settled and every update that
    // arrived for it was dropped. The log's own `state` is the only lifecycle.
    const sid = "s-reasoning";
    const reasoning: Item = {
      type: "reasoning",
      id: "rs_1",
      summary: [{ type: "summary_text", text: "thinking" }],
    };
    useMessageStore.getState().onBufferItem(sid, {
      session_id: sid,
      seq: 7,
      kind: "item/assistant",
      state: "in_progress",
      body: reasoning,
    });
    const live = useMessageStore.getState().bySession.get(sid)!.messages[0]!;
    expect(live.state).toBe("in_progress");

    useMessageStore.getState().onBufferItem(sid, {
      session_id: sid,
      seq: 7,
      kind: "item/assistant",
      state: "final",
      body: {
        ...reasoning,
        summary: [{ type: "summary_text", text: "thought it through" }],
      },
    });
    const settled = useMessageStore.getState().bySession.get(sid)!.messages[0]!;
    expect(settled.state).toBe("final");
    expect(itemPlainText(settled.body as Item)).toBe("thought it through");
  });

  it("rejects a row that does not say what state it is in", () => {
    // Guessing "settled" for a missing state is what silently swallowed live
    // updates, so the wire has to say.
    const sid = "s-nostate";
    useMessageStore.getState().onBufferItem(sid, {
      session_id: sid,
      seq: 1,
      kind: "item/assistant",
      body: assistantMsg("msg_1", "unlabelled"),
    } as never);
    expect(
      useMessageStore.getState().bySession.get(sid)?.messages ?? [],
    ).toHaveLength(0);
  });

  it("buffer/item replaces live content on the same seq (no merge)", () => {
    const sid = "s-replace";
    markTurnRunning(sid);
    useMessageStore.getState().onBufferItem(sid, {
      session_id: sid,
      seq: 3,
      kind: "item/assistant",
      state: "in_progress",
      body: assistantMsg("msg_r", "partial", "in_progress"),
    });
    useMessageStore.getState().onBufferItem(sid, {
      session_id: sid,
      seq: 3,
      kind: "item/assistant",
      state: "final",

      body: assistantMsg("msg_r", "final from ledger", "completed"),
    });
    const row = useMessageStore.getState().bySession.get(sid)!.messages[0]!;
    expect(itemPlainText(row.body as Item)).toBe("final from ledger");
    expect(row.state).toBe("final");
  });

  it("a settled seq keeps its text when a later row reuses the provider id", () => {
    const sid = "s-g4";
    load(sid, [
      ev(0, assistantMsg("msg_1", "old reply")),
      ev(1, userMsg("rolled-up")),
    ]);
    useMessageStore.getState().onBufferItem(sid, {
      session_id: sid,
      seq: 2,
      kind: "item/assistant",
      state: "final",
      body: assistantMsg("msg_1", "new reply"),
    });
    const slice = useMessageStore.getState().bySession.get(sid)!;
    expect(
      itemPlainText(slice.messages.find((m) => m.seq === 0)!.body as Item),
    ).toBe("old reply");
    expect(
      itemPlainText(slice.messages.find((m) => m.seq === 2)!.body as Item),
    ).toBe("new reply");
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
      kind: "item/user",
      state: "final",

      body: userMsg("hello"),
    });
    slice = useMessageStore.getState().bySession.get(sid)!;
    expect(slice.pendingUser).toBeNull();
    expect(slice.messages).toHaveLength(1);
    expect(slice.messages[0]!.seq).toBe(0);
  });

  it("pending user also seals on a plan/execute row (mark replaces the bubble)", () => {
    const sid = "s-pend-plan";
    useMessageStore.getState().pushPendingUser(sid, {
      clientId: "pending-plan",
      item: userMsg("按当前计划开始执行。"),
    });
    useMessageStore.getState().onBufferItem(sid, {
      session_id: sid,
      seq: 0,
      kind: "plan/execute",
      state: "final",
      body: userMsg("按当前计划开始执行。"),
    });
    const slice = useMessageStore.getState().bySession.get(sid)!;
    expect(slice.pendingUser).toBeNull();
    expect(slice.messages).toHaveLength(1);
    expect(slice.messages[0]!.kind).toBe("plan/execute");
  });

  it("displayMessages snapshot is stable while pending user is set", () => {
    const sid = "s-new-pending";
    useMessageStore.getState().pushPendingUser(sid, {
      clientId: "c1",
      item: userMsg("hi"),
    });
    const slice = useMessageStore.getState().bySession.get(sid)!;
    expect(displayMessages(slice)).toBe(displayMessages(slice));
    expect(displayMessages(undefined)).toBe(displayMessages(undefined));
  });

  it("subscribing to displayMessages does not trip max update depth on a new session send", () => {
    const sid = "s-new-hook";
    useMessageStore.getState().pushPendingUser(sid, {
      clientId: "c1",
      item: userMsg("hi"),
    });
    function Probe() {
      const rows = useMessageStore((s) =>
        displayMessages(s.bySession.get(sid)),
      );
      return createElement("div", { "data-testid": "n" }, String(rows.length));
    }
    expect(() => render(createElement(Probe))).not.toThrow();
    expect(screen.getByTestId("n").textContent).toBe("1");
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
    expect(
      useMessageStore.getState().bySession.get(sid)!.pendingUser?.clientId,
    ).toBe("a");
  });

  it("finalizeTurn does not drop seq rows", () => {
    const sid = "s-fin";
    markTurnRunning(sid);
    useMessageStore.getState().onBufferItem(sid, {
      session_id: sid,
      seq: 1,
      kind: "item/assistant",
      state: "final",

      body: assistantMsg("msg_1", "mid", "in_progress"),
    });
    const slice = useMessageStore.getState().bySession.get(sid)!;
    expect(slice.messages).toHaveLength(1);
    expect(slice.messages[0]!.seq).toBe(1);
    expect(slice.messages[0]!.state).toBe("final");
  });

  it("hydrates userDetailBefore from a partial buffer/load window", () => {
    const sid = "s-partial-k";
    load(
      sid,
      [ev(10, userMsg("later")), ev(11, assistantMsg("a", "ok"))],
      10,
      12,
      3,
    );
    const slice = useMessageStore.getState().bySession.get(sid)!;
    expect(slice.fromSeq).toBe(10);
    expect(slice.userDetailBefore).toBe(3);
    expect(deriveUserAnchorK(slice.messages, 0, slice.userDetailBefore)).toBe(
      3,
    );
  });

  it("malformed buffer/item missing kind/body does not overwrite a valid seq", () => {
    const sid = "s-malformed";
    load(sid, [ev(3, assistantMsg("msg_keep", "keep"))]);
    useMessageStore.getState().onBufferItem(sid, {
      session_id: sid,
      seq: 3,
      item: assistantMsg("msg_keep", "stale legacy"),
    } as never);
    const slice = useMessageStore.getState().bySession.get(sid)!;
    expect(itemPlainText(slice.messages[0]!.body as Item)).toBe("keep");
    expect(slice.shapeError).toMatch(/kind\/body/);
  });

  it("malformed buffer/item without an existing seq is ignored", () => {
    const sid = "s-malformed-new";
    useMessageStore.getState().onBufferItem(sid, {
      session_id: sid,
      seq: 9,
      item: assistantMsg("ghost", "nope"),
    } as never);
    const slice = useMessageStore.getState().bySession.get(sid)!;
    expect(slice.messages).toHaveLength(0);
    expect(slice.shapeError).toBeNull();
  });

  it("buffer/reverted keeps only the surviving tail when next_seq stays ahead", () => {
    const sid = "s-rev";
    load(sid, [
      ev(0, userMsg("a")),
      ev(1, assistantMsg("x", "b")),
      ev(2, userMsg("c")),
    ]);
    // A truncate drops the live tail to seq 0 but never rewinds the allocator:
    // the next append still takes seq 5, so rows 1..4 are gone for good and must
    // not survive in the window.
    useMessageStore.getState().onBufferReverted(sid, {
      session_id: sid,
      last_seq: 0,
      next_seq: 5,
    });
    const slice = useMessageStore.getState().bySession.get(sid)!;
    expect(slice.messages.map((r) => r.seq)).toEqual([0]);
    expect(slice.toSeq).toBe(5);
  });

  it("ensureSeqLoaded is a no-op when the seq is already in the window", async () => {
    const sid = "s-in-window";
    load(sid, [ev(0, userMsg("a")), ev(1, assistantMsg("x", "b"))], 0, 2);
    const sendRpc = vi.fn();
    useConnectionStore.setState({ sendRpc } as never);
    await expect(
      useMessageStore.getState().ensureSeqLoaded(sid, 1),
    ).resolves.toBe(true);
    expect(sendRpc).not.toHaveBeenCalled();
  });

  it("ensureSeqLoaded pages backward until the target seq is in the store", async () => {
    const sid = "s-history";
    load(sid, [ev(40, userMsg("tail"))], 40, 41);
    const sendRpc = vi.fn(
      async (_method: string, params?: Record<string, unknown>) => ({
        session_id: sid,
        from_seq: params?.from_seq,
        to_seq: params?.to_seq,
        events: [ev(5, userMsg("older"))],
      }),
    );
    useConnectionStore.setState({ sendRpc } as never);
    await expect(
      useMessageStore.getState().ensureSeqLoaded(sid, 5),
    ).resolves.toBe(true);
    expect(sendRpc).toHaveBeenCalled();
    expect(useMessageStore.getState().bySession.get(sid)!.bySeq.has(5)).toBe(
      true,
    );
    expect(useMessageStore.getState().bySession.get(sid)!.fromSeq).toBe(0);
  });

  it("ensureSeqLoaded follows only the latest request", async () => {
    const sid = "s-cancel";
    load(sid, [ev(40, userMsg("tail"))], 40, 41);
    let resume: ((v: BufferLoaded) => void) | undefined;
    const sendRpc = vi.fn(
      () =>
        new Promise<BufferLoaded>((resolve) => {
          resume = resolve;
        }),
    );
    useConnectionStore.setState({ sendRpc } as never);
    let currentGen = 1;
    const first = useMessageStore
      .getState()
      .ensureSeqLoaded(sid, 1, () => currentGen === 1);
    currentGen = 2;
    resume?.({
      session_id: sid,
      from_seq: 0,
      to_seq: 40,
      events: [ev(1, userMsg("old"))],
    });
    await expect(first).resolves.toBe(false);
  });

  it("ensureSeqLoaded returns false when the seq was reverted out of the window", async () => {
    const sid = "s-gone";
    load(sid, [ev(0, userMsg("keep"))], 0, 1);
    useConnectionStore.setState({ sendRpc: vi.fn() } as never);
    await expect(
      useMessageStore.getState().ensureSeqLoaded(sid, 4),
    ).resolves.toBe(false);
  });
});

describe("messageStore buffer window — tail append (P6 catch-up)", () => {
  const sid = "s-window";

  beforeEach(() => {
    useMessageStore.setState({ bySession: new Map() });
  });

  it("keeps the window start and user-detail count when a load only appends the tail", () => {
    load(
      sid,
      [ev(0, userMsg("ask")), ev(1, assistantMsg("a", "first"))],
      0,
      2,
      0,
    );

    // Gap catch-up: the retained window [0,2) is topped up with [2,3).
    load(sid, [ev(2, assistantMsg("b", "second"))], 2, 3, 7);

    const slice = useMessageStore.getState().bySession.get(sid)!;
    expect(slice.messages.map((r) => r.seq)).toEqual([0, 1, 2]);
    expect(slice.toSeq).toBe(3);
    // The older rows are still held: the start must not jump to the appended tail.
    expect(slice.fromSeq).toBe(0);
    // `userDetailBefore` counts from the window start, not from the tail base.
    expect(slice.userDetailBefore).toBe(0);
  });

  it("still moves the window start for a history page loaded below the window", () => {
    load(
      sid,
      [ev(10, userMsg("ask")), ev(11, assistantMsg("a", "first"))],
      10,
      12,
      3,
    );

    load(sid, [ev(8, userMsg("older"))], 8, 10, 1);

    const slice = useMessageStore.getState().bySession.get(sid)!;
    expect(slice.messages.map((r) => r.seq)).toEqual([8, 10, 11]);
    expect(slice.fromSeq).toBe(8);
    expect(slice.userDetailBefore).toBe(1);
    expect(slice.toSeq).toBe(12);
  });
});

describe("sessionStore.applySnapshot — retained window catch-up (P6)", () => {
  const sid = "child-a";

  function snapshot(nextSeq: number): SessionSnapshot {
    return {
      session_id: sid,
      project: "/p",
      agent_id: "default",
      api_model_id: "m",
      buffer: { last_seq: nextSeq, next_seq: nextSeq, revision: 0 },
      turn: null,
    };
  }

  function seedRpc(events: WireBufferEvent[] = []): ReturnType<typeof vi.fn> {
    const sendRpc = vi.fn(async (method: string, params: never) => {
      if (method !== "buffer/load") return {};
      const p = params as unknown as { from_seq: number; to_seq: number };
      const inside = events.filter(
        (e) => e.seq >= p.from_seq && e.seq < p.to_seq,
      );
      return {
        session_id: sid,
        from_seq: p.from_seq,
        to_seq: p.to_seq,
        events: inside,
      } satisfies BufferLoaded;
    });
    useConnectionStore.setState({ state: "connected", sendRpc } as never);
    return sendRpc;
  }

  beforeEach(() => {
    useMessageStore.setState({ bySession: new Map() });
    useTurnStore.setState({ byId: new Map() });
  });

  it("appends the gap when a retained window lags the snapshot next_seq", () => {
    const sendRpc = seedRpc([ev(5, assistantMsg("e", "appended"))]);
    // Retained window from an earlier expand: [0,5), server is at 8.
    load(sid, [ev(0, userMsg("ask"))], 0, 5, 0);

    useSessionStore.getState().applySnapshot(snapshot(8));

    expect(sendRpc).toHaveBeenCalledWith("buffer/load", {
      from_seq: 5,
      to_seq: 8,
      session_id: sid,
    });
  });

  it("cold-starts when there is no retained window at all", () => {
    const sendRpc = seedRpc();

    useSessionStore.getState().applySnapshot(snapshot(8));

    expect(sendRpc).toHaveBeenCalledWith("buffer/load", {
      from_seq: 0,
      to_seq: 8,
      session_id: sid,
    });
  });

  it("does not re-fetch when the retained window already covers next_seq", () => {
    const sendRpc = seedRpc();
    load(sid, [ev(0, userMsg("ask"))], 0, 8, 0);

    useSessionStore.getState().applySnapshot(snapshot(8));

    expect(sendRpc).not.toHaveBeenCalled();
  });
});

describe("messageStore queued-batch in-flight bubble", () => {
  beforeEach(() => {
    useMessageStore.setState({ bySession: new Map() });
  });

  it("mirrors the batch joined exactly like the server's merged row", () => {
    useMessageStore.getState().setPendingQueue("s-q", ["one", "two"]);
    const slice = useMessageStore.getState().bySession.get("s-q")!;
    expect(slice.pendingQueue?.joined).toBe("one\n\ntwo");
    expect(slice.pendingQueue?.texts).toEqual(["one", "two"]);
    // Chrome, not a log row: nothing lands in the transcript.
    expect(slice.messages).toHaveLength(0);
    expect(slice.display).toHaveLength(0);
  });

  it("the durable merged row seals the bubble and stays as a normal message", () => {
    useMessageStore.getState().setPendingQueue("s-q2", ["one", "two"]);
    useMessageStore.getState().onBufferItem("s-q2", {
      session_id: "s-q2",
      seq: 0,
      kind: "item/user",
      state: "final",
      body: userMsg("one\n\ntwo"),
    });
    const slice = useMessageStore.getState().bySession.get("s-q2")!;
    expect(slice.pendingQueue).toBeNull();
    expect(slice.messages).toHaveLength(1);
    expect(itemPlainText(slice.messages[0]!.body as Item)).toBe("one\n\ntwo");
  });

  it("a row with other text never seals it", () => {
    useMessageStore.getState().setPendingQueue("s-q3", ["queued"]);
    useMessageStore.getState().onBufferItem("s-q3", {
      session_id: "s-q3",
      seq: 0,
      kind: "item/assistant",
      state: "final",
      body: assistantMsg("msg_1", "done"),
    });
    expect(
      useMessageStore.getState().bySession.get("s-q3")?.pendingQueue?.joined,
    ).toBe("queued");
  });

  it("an explicit clear removes it, and re-setting the same text is a no-op", () => {
    useMessageStore.getState().setPendingQueue("s-q4", ["a"]);
    const first = useMessageStore.getState().bySession.get("s-q4")!.pendingQueue;
    useMessageStore.getState().setPendingQueue("s-q4", ["a"]);
    expect(useMessageStore.getState().bySession.get("s-q4")!.pendingQueue).toBe(
      first,
    );
    useMessageStore.getState().setPendingQueue("s-q4", null);
    expect(useMessageStore.getState().bySession.get("s-q4")!.pendingQueue).toBeNull();
    // Clearing again stays quiet.
    useMessageStore.getState().setPendingQueue("s-q4", []);
    expect(useMessageStore.getState().bySession.get("s-q4")!.pendingQueue).toBeNull();
  });

  it("names the settling row by seq and clears it once the settle has played", () => {
    useMessageStore.getState().setPendingQueue("s-q5", ["one", "two"]);
    useMessageStore.getState().onBufferItem("s-q5", {
      session_id: "s-q5",
      seq: 4,
      kind: "item/user",
      state: "final",
      body: userMsg("one\n\ntwo"),
    });
    let slice = useMessageStore.getState().bySession.get("s-q5")!;
    // One update: the bubble goes and this exact row takes over from it.
    expect(slice.pendingQueue).toBeNull();
    expect(slice.landedQueueSeq).toBe(4);

    useMessageStore.getState().clearLandedQueueSeq("s-q5");
    slice = useMessageStore.getState().bySession.get("s-q5")!;
    expect(slice.landedQueueSeq).toBeNull();
  });

  it("no settle marker for a row that does not seal the batch", () => {
    useMessageStore.getState().setPendingQueue("s-q6", ["queued"]);
    useMessageStore.getState().onBufferItem("s-q6", {
      session_id: "s-q6",
      seq: 0,
      kind: "item/assistant",
      state: "final",
      body: assistantMsg("msg_1", "done"),
    });
    expect(
      useMessageStore.getState().bySession.get("s-q6")!.landedQueueSeq,
    ).toBeNull();
  });

  it("a new batch is a new arrival, not the old settle", () => {
    useMessageStore.getState().setPendingQueue("s-q7", ["first"]);
    useMessageStore.getState().onBufferItem("s-q7", {
      session_id: "s-q7",
      seq: 0,
      kind: "item/user",
      state: "final",
      body: userMsg("first"),
    });
    expect(useMessageStore.getState().bySession.get("s-q7")!.landedQueueSeq).toBe(
      0,
    );

    useMessageStore.getState().setPendingQueue("s-q7", ["second"]);
    const slice = useMessageStore.getState().bySession.get("s-q7")!;
    expect(slice.pendingQueue?.joined).toBe("second");
    expect(slice.landedQueueSeq).toBeNull();
  });

  it("a revert drops the bubble and the settle it was waiting for", () => {
    useMessageStore.getState().setPendingQueue("s-q8", ["discarded"]);
    useMessageStore.getState().onBufferReverted("s-q8", {
      session_id: "s-q8",
      last_seq: -1,
      next_seq: 3,
    });
    const slice = useMessageStore.getState().bySession.get("s-q8")!;
    // The row this bubble was waiting for can never land: keeping it would leave
    // a permanent "pending" phantom in the transcript.
    expect(slice.pendingQueue).toBeNull();
    expect(slice.landedQueueSeq).toBeNull();
  });
});

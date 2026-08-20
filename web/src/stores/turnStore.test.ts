import { beforeEach, describe, expect, it, vi } from "vitest";

import { isCompactCutRow, itemPlainText } from "../api/adapter";
import type { SessionSnapshot, TurnFinished, TurnSnapshot } from "../api/types";
import { useConnectionStore } from "./connectionStore";
import { useMessageStore } from "./messageStore";
import { useNotificationStore } from "./notificationStore";
import { useToastStore } from "./toastStore";
import { EMPTY_SLICE, shouldApplyTurnEnd, useTurnStore } from "./turnStore";

vi.stubGlobal(
  "window",
  Object.assign(globalThis, {
    requestAnimationFrame: (fn: FrameRequestCallback) => {
      fn(0);
      return 0;
    },
    cancelAnimationFrame: () => {},
  }),
);

function snapshot(sessionId: string): SessionSnapshot {
  return {
    session_id: sessionId,
    project: "/proj",
    agent_id: "default",
    api_model_id: "m",
    buffer: { last_seq: -1, next_seq: 0, revision: 0 },
    turn: null,
    context_window: 0,
  };
}

function turnSnap(turnId: string): TurnSnapshot {
  return {
    turn_id: turnId,
    phase: "calling_llm",
    step: 1,
    step_max: 5,
    started_at_ms: 1,
  };
}

describe("turnStore convergence", () => {
  beforeEach(() => {
    useTurnStore.setState({ byId: new Map() });
    useMessageStore.setState({ bySession: new Map() });
    useNotificationStore.setState({ bySession: new Map() });
    useToastStore.setState({ toasts: [] });
    useConnectionStore.setState({
      sendRpc: vi.fn(async () => ({ started: true })),
    } as never);
  });

  it("starts manual compact only when the server marks the session eligible", () => {
    const sessionId = "s-compact";
    const sendRpc = vi.fn(async () => ({ accepted: true, operation_id: "op-1" }));
    useConnectionStore.setState({ sendRpc } as never);
    useTurnStore.setState({
      byId: new Map([
        [
          sessionId,
          {
            ...EMPTY_SLICE,
            contextWindow: 1000,
            contextTokensEstimate: 300,
            compactEligible: true,
          },
        ],
      ]),
    });

    useTurnStore.getState().compact(sessionId);

    expect(useTurnStore.getState().byId.get(sessionId)?.compacting).toBe(true);
    expect(sendRpc).toHaveBeenCalledWith("session/compact", {
      session_id: sessionId,
    });
  });

  it("onCompactLifecycle started/succeeded drives compacting for auto and manual", () => {
    const sessionId = "s-life";
    useTurnStore.setState({
      byId: new Map([[sessionId, { ...EMPTY_SLICE, runState: "running" }]]),
    });
    const snap = snapshot(sessionId);
    useTurnStore.getState().onCompactLifecycle({
      session_id: sessionId,
      trigger: "auto",
      stage: "started",
      snapshot: { ...snap, compacting: false },
    });
    expect(useTurnStore.getState().byId.get(sessionId)?.compacting).toBe(true);
    expect(useTurnStore.getState().byId.get(sessionId)?.turnPhase).toBe("compacting");

    useTurnStore.getState().onCompactLifecycle({
      session_id: sessionId,
      trigger: "auto",
      stage: "succeeded",
      snapshot: { ...snap, compacting: false },
    });
    expect(useTurnStore.getState().byId.get(sessionId)?.compacting).toBe(false);
  });

  it("shouldApplyTurnEnd skips duplicate idle and a newer live turn", () => {
    expect(shouldApplyTurnEnd(null, "idle", "t1")).toBe(false);
    expect(shouldApplyTurnEnd(null, "running", "t1")).toBe(false);
    expect(shouldApplyTurnEnd("t2", "running", "t1")).toBe(false);
    expect(shouldApplyTurnEnd("t1", "running", "t1")).toBe(true);
    expect(shouldApplyTurnEnd("t1", "cancelling", "t1")).toBe(true);
    expect(shouldApplyTurnEnd("t1", "running", null)).toBe(true);
  });

  it("onTurnFinished with a different live turn_id does not force idle", () => {
    const sessionId = "s1";
    useTurnStore.setState({
      byId: new Map([
        [
          sessionId,
          {
            ...EMPTY_SLICE,
            runState: "running",
            currentTurnId: "t-next",
            turnPhase: "calling_llm",
            turnStep: 1,
            turnStepMax: 5,
          },
        ],
      ]),
    });

    const tf: TurnFinished = {
      turn_id: "t-old",
      reason: "completed",
      final_text: "done",
      error: null,
      snapshot: snapshot(sessionId),
      session_id: sessionId,
    };
    useTurnStore.getState().onTurnFinished(tf);

    const slice = useTurnStore.getState().byId.get(sessionId)!;
    expect(slice.runState).toBe("running");
    expect(slice.currentTurnId).toBe("t-next");
  });

  it("onTurnFinished matching turn_id idles and drops unsealed live overlay", () => {
    const sessionId = "s-finish-drop";
    useTurnStore.setState({
      byId: new Map([
        [
          sessionId,
          {
            ...EMPTY_SLICE,
            runState: "running",
            currentTurnId: "t1",
          },
        ],
      ]),
    });
    useMessageStore.getState().applyStreamEvent(sessionId, "t1", 1, {
      type: "response.reasoning_text.delta",
      sequence_number: 1,
      item_id: "rs_half",
      output_index: 0,
      content_index: 0,
      delta: "half",
    });

    useTurnStore.getState().onTurnFinished({
      turn_id: "t1",
      reason: "error",
      final_text: "stream ended",
      error: { code: "internal", message: "stream ended" },
      snapshot: snapshot(sessionId),
      session_id: sessionId,
    });

    const turn = useTurnStore.getState().byId.get(sessionId)!;
    expect(turn.runState).toBe("idle");
    expect(turn.currentTurnId).toBeNull();
    const msgs = useMessageStore.getState().bySession.get(sessionId)!;
    expect(msgs.messages.every((m) => m.seq >= 0)).toBe(true);
    expect(msgs.turnEndNotice?.message).toBe("stream ended");
  });

  it("applySnapshotTurn with null forces idle", () => {
    const sessionId = "s2";
    useTurnStore.getState().applySnapshotTurn(sessionId, turnSnap("t1"));
    expect(useTurnStore.getState().byId.get(sessionId)!.runState).toBe("running");

    useTurnStore.getState().applySnapshotTurn(sessionId, null);
    const slice = useTurnStore.getState().byId.get(sessionId)!;
    expect(slice.runState).toBe("idle");
    expect(slice.currentTurnId).toBeNull();
  });

  it("applySnapshotMeter hydrates context ring from provider stats only", () => {
    const sessionId = "s2-meter";
    useTurnStore.getState().applySnapshotMeter(sessionId, {
      ...snapshot(sessionId),
      context_window: 256000,
      last_turn_token_stats: {
        prompt_tokens: 1000,
        completion_tokens: 50,
        cache_hit_tokens: 800,
        cache_miss_tokens: 200,
      },
      cumulative_token_stats: {
        prompt_tokens: 9000,
        completion_tokens: 450,
        cache_hit_tokens: 7200,
        cache_miss_tokens: 1800,
      },
    });
    const slice = useTurnStore.getState().byId.get(sessionId)!;
    expect(slice.contextWindow).toBe(256000);
    expect(slice.lastTurnPromptTokens).toBe(1000);
    expect(slice.lastTurnCacheHitTokens).toBe(800);
    expect(slice.lastTurnCacheMissTokens).toBe(200);
    // Session-total accumulators hydrate from cumulative_token_stats.
    expect(slice.sessionPromptTokens).toBe(9000);
    expect(slice.sessionCacheHitTokens).toBe(7200);
    expect(slice.sessionCacheMissTokens).toBe(1800);
  });

  it("llm_completed events accumulate session totals; turn_finished overwrites with snapshot", () => {
    const sessionId = "s2-acc";
    useTurnStore.getState().onTurnStarted({
      session_id: sessionId,
      turn_id: "t1",
      input: "hi",
      step_max: 5,
    });
    // First request: 800 hit / 200 miss.
    useTurnStore.getState().onTurnEvent({
      session_id: sessionId,
      turn_id: "t1",
      event: {
        type: "llm_completed",
        prompt_tokens: 1000,
        completion_tokens: 50,
        cache_hit_tokens: 800,
        cache_miss_tokens: 200,
        stop_reason: "stop",
      },
    });
    // Second request (tool loop step): 600 hit / 400 miss → totals accumulate.
    useTurnStore.getState().onTurnEvent({
      session_id: sessionId,
      turn_id: "t1",
      event: {
        type: "llm_completed",
        prompt_tokens: 1000,
        completion_tokens: 30,
        cache_hit_tokens: 600,
        cache_miss_tokens: 400,
        stop_reason: "stop",
      },
    });
    let slice = useTurnStore.getState().byId.get(sessionId)!;
    expect(slice.lastTurnCacheHitTokens).toBe(600); // last request only
    expect(slice.sessionPromptTokens).toBe(2000);
    expect(slice.sessionCacheHitTokens).toBe(1400);
    expect(slice.sessionCacheMissTokens).toBe(600);

    // Turn finished: backend snapshot cum (e.g. 2000/1400/600) overwrites.
    useTurnStore.getState().onTurnFinished({
      session_id: sessionId,
      turn_id: "t1",
      reason: "completed",
      final_text: "done",
      error: null,
      snapshot: {
        ...snapshot(sessionId),
        cumulative_token_stats: {
          prompt_tokens: 2000,
          completion_tokens: 80,
          cache_hit_tokens: 1400,
          cache_miss_tokens: 600,
        },
      },
    });
    slice = useTurnStore.getState().byId.get(sessionId)!;
    expect(slice.sessionCacheHitTokens).toBe(1400);
    expect(slice.sessionCacheMissTokens).toBe(600);
    expect(slice.runState).toBe("idle");
  });

  it("applySnapshotMeter without stats leaves prompt truth absent", () => {
    const sessionId = "s2-meter-absent";
    useTurnStore.getState().applySnapshotMeter(sessionId, {
      ...snapshot(sessionId),
      context_window: 128000,
    });
    const slice = useTurnStore.getState().byId.get(sessionId)!;
    expect(slice.contextWindow).toBe(128000);
    expect(slice.lastTurnPromptTokens).toBe(0);
  });

  it("applySnapshotMeter clears stale last-turn prompt after compact-style snapshot", () => {
    const sessionId = "s2-meter-clear";
    useTurnStore.setState({
      byId: new Map([
        [
          sessionId,
          {
            ...EMPTY_SLICE,
            lastTurnPromptTokens: 50000,
            lastTurnCacheHitTokens: 40000,
            contextTokensEstimate: 50000,
          },
        ],
      ]),
    });
    useTurnStore.getState().applySnapshotMeter(sessionId, {
      ...snapshot(sessionId),
      context_window: 128000,
      context_tokens_estimate: 12000,
      compact_eligible: false,
      // Post-compact snapshots omit last_turn_token_stats.
    });
    const slice = useTurnStore.getState().byId.get(sessionId)!;
    expect(slice.lastTurnPromptTokens).toBe(0);
    expect(slice.lastTurnCacheHitTokens).toBe(0);
    expect(slice.contextTokensEstimate).toBe(12000);
  });

  it("applySnapshotMeter hydrates todos from snapshot", () => {
    const sessionId = "s-todo-hydrate";
    useTurnStore.getState().applySnapshotMeter(sessionId, {
      ...snapshot(sessionId),
      todos: [
        { id: "t1", content: "ship", status: "in_progress" },
        { id: "t2", content: "later", status: "pending" },
      ],
    });
    const slice = useTurnStore.getState().byId.get(sessionId)!;
    expect(slice.todoInProgress).toBe(1);
    expect(slice.todoPending).toBe(1);
    expect(slice.todoItems.map((t) => t.content)).toEqual(["ship", "later"]);
  });

  it("applySnapshotMeter without todos leaves existing overlay", () => {
    const sessionId = "s-todo-keep";
    useTurnStore.setState({
      byId: new Map([
        [
          sessionId,
          {
            ...EMPTY_SLICE,
            todoItems: [{ id: "t1", content: "keep", status: "pending" }],
            todoPending: 1,
          },
        ],
      ]),
    });
    useTurnStore.getState().applySnapshotMeter(sessionId, snapshot(sessionId));
    const slice = useTurnStore.getState().byId.get(sessionId)!;
    expect(slice.todoItems).toEqual([
      { id: "t1", content: "keep", status: "pending" },
    ]);
    expect(slice.todoPending).toBe(1);
  });

  it("todo_progress with empty items retains struck-through history but clears counts", () => {
    const sessionId = "s-todo-retain";
    useTurnStore.setState({
      byId: new Map([
        [
          sessionId,
          {
            ...EMPTY_SLICE,
            currentTurnId: "t1",
            runState: "running",
            todoItems: [
              { id: "t1", content: "ship", status: "in_progress" },
              { id: "t2", content: "later", status: "pending" },
            ],
            todoPending: 1,
            todoInProgress: 1,
          },
        ],
      ]),
    });
    useTurnStore.getState().onTurnEvent({
      session_id: sessionId,
      turn_id: "t1",
      event: {
        type: "todo_progress",
        pending: 0,
        in_progress: 0,
        completed: 0,
        items: [],
      },
    });
    const slice = useTurnStore.getState().byId.get(sessionId)!;
    expect(slice.todoItems).toEqual([
      { id: "t1", content: "ship", status: "completed" },
      { id: "t2", content: "later", status: "completed" },
    ]);
    expect(slice.todoPending).toBe(0);
    expect(slice.todoInProgress).toBe(0);
    expect(slice.todoCompleted).toBe(0);
  });

  it("applySnapshotMeter with empty todos retains struck-through history but clears counts", () => {
    const sessionId = "s-todo-snap-retain";
    useTurnStore.setState({
      byId: new Map([
        [
          sessionId,
          {
            ...EMPTY_SLICE,
            todoItems: [{ id: "t1", content: "ship", status: "completed" }],
            todoCompleted: 1,
          },
        ],
      ]),
    });
    useTurnStore.getState().applySnapshotMeter(sessionId, {
      ...snapshot(sessionId),
      todos: [],
    });
    const slice = useTurnStore.getState().byId.get(sessionId)!;
    expect(slice.todoItems).toEqual([
      { id: "t1", content: "ship", status: "completed" },
    ]);
    expect(slice.todoCompleted).toBe(0);
  });

  it("todo_progress with empty items and no history stays empty", () => {
    const sessionId = "s-todo-empty";
    useTurnStore.setState({
      byId: new Map([
        [sessionId, { ...EMPTY_SLICE, currentTurnId: "t1", runState: "running" }],
      ]),
    });
    useTurnStore.getState().onTurnEvent({
      session_id: sessionId,
      turn_id: "t1",
      event: {
        type: "todo_progress",
        pending: 0,
        in_progress: 0,
        completed: 0,
        items: [],
      },
    });
    const slice = useTurnStore.getState().byId.get(sessionId)!;
    expect(slice.todoItems).toEqual([]);
    expect(slice.todoCompleted).toBe(0);
  });

  it("agent/run reject returns to idle and discards the optimistic user row", async () => {
    const sessionId = "s3";
    useConnectionStore.setState({
      sendRpc: vi.fn(async () => {
        throw new Error("agent already running");
      }),
    } as never);

    const started = useTurnStore.getState().start(sessionId, "hello");
    expect(started).toBe(true);
    expect(useTurnStore.getState().byId.get(sessionId)!.runState).toBe("running");
    expect(useMessageStore.getState().bySession.get(sessionId)!.pendingUser).toBeTruthy();

    await vi.waitFor(() => {
      expect(useTurnStore.getState().byId.get(sessionId)!.runState).toBe("idle");
    });
    expect(useMessageStore.getState().bySession.get(sessionId)?.pendingUser).toBeNull();
  });
});

describe("grantPermission receipt (FE-04)", () => {
  function pendingSlice(sessionId: string) {
    useTurnStore.setState({
      byId: new Map([
        [
          sessionId,
          {
            ...EMPTY_SLICE,
            pendingPermission: {
              turn_id: "t1",
              request_id: "req-1",
              tool: "bash",
              rule_id: "default",
              summary: "Run bash",
            },
          },
        ],
      ]),
    });
  }

  function deferred<T>() {
    let resolve!: (value: T) => void;
    let reject!: (err: unknown) => void;
    const promise = new Promise<T>((res, rej) => {
      resolve = res;
      reject = rej;
    });
    return { promise, resolve, reject };
  }

  it("keeps the card open until the agent/permission receipt arrives, then closes", async () => {
    const sessionId = "s-grant-ok";
    const { promise, resolve } = deferred<{ ok: boolean }>();
    useConnectionStore.setState({
      sendRpc: vi.fn(() => promise),
    } as never);
    pendingSlice(sessionId);

    useTurnStore.getState().grantPermission(sessionId, true, false);
    // Card is still open while awaiting the receipt.
    expect(useTurnStore.getState().byId.get(sessionId)!.pendingPermission).not.toBeNull();

    resolve({ ok: true });
    await vi.waitFor(() => {
      expect(useTurnStore.getState().byId.get(sessionId)!.pendingPermission).toBeNull();
    });
  });

  it("rolls the card back and surfaces an explicit error when the receipt fails", async () => {
    const sessionId = "s-grant-fail";
    const { promise, reject } = deferred<{ ok: boolean }>();
    useConnectionStore.setState({
      sendRpc: vi.fn(() => promise),
    } as never);
    pendingSlice(sessionId);

    useTurnStore.getState().grantPermission(sessionId, true, false);
    expect(useTurnStore.getState().byId.get(sessionId)!.pendingPermission).not.toBeNull();

    reject(new Error("permission rejected"));
    await vi.waitFor(() => {
      // Card is restored (rollback) and the error is not silently swallowed.
      expect(
        useTurnStore.getState().byId.get(sessionId)!.pendingPermission?.request_id,
      ).toBe("req-1");
    });
  });

  it("routes turn error and snapshot warning to toast, not the bell", () => {
    const sessionId = "s-toast-fail";
    useTurnStore.getState().onTurnStarted({
      session_id: sessionId,
      turn_id: "t-fail",
      input: "hi",
      step_max: 5,
    });

    useTurnStore.getState().onTurnEvent({
      session_id: sessionId,
      turn_id: "t-fail",
      event: { type: "error", code: "internal", message: "LLM request failed" },
    });
    useTurnStore.getState().onTurnEvent({
      session_id: sessionId,
      turn_id: "t-fail",
      event: {
        type: "snapshot_notice",
        level: "warn",
        message: "Workspace snapshot track failed",
      },
    });

    expect(useNotificationStore.getState().bySession.size).toBe(0);
    const toasts = useToastStore.getState().toasts.map((t) => t.message);
    expect(toasts).toContain("LLM request failed");
    expect(toasts).toContain("Workspace snapshot track failed");
  });

  it("onTurnStarted hydrates a user row for server-initiated input", () => {
    const sessionId = "s-idle-kill";
    useTurnStore.getState().onTurnStarted({
      session_id: sessionId,
      turn_id: "t-idle",
      input: "<system-reminder>bash exited</system-reminder>",
      step_max: 8,
    });
    const slice = useTurnStore.getState().byId.get(sessionId)!;
    expect(slice.runState).toBe("running");
    expect(slice.currentTurnId).toBe("t-idle");
    const rows = useMessageStore.getState().bySession.get(sessionId)?.messages ?? [];
    expect(rows).toHaveLength(0);
    const pending = useMessageStore.getState().bySession.get(sessionId)?.pendingUser;
    expect(pending).toBeTruthy();
    expect(itemPlainText(pending!.item)).toBe(
      "<system-reminder>bash exited</system-reminder>",
    );

    useTurnStore.getState().onTurnStarted({
      session_id: sessionId,
      turn_id: "t-idle",
      input: "<system-reminder>bash exited</system-reminder>",
      step_max: 8,
    });
    const again = useMessageStore.getState().bySession.get(sessionId)?.messages ?? [];
    expect(again).toHaveLength(0);
    expect(useMessageStore.getState().bySession.get(sessionId)?.pendingUser).toBeTruthy();
  });

  it("onTurnStarted does not duplicate an optimistic start() user row", () => {
    const sessionId = "s-human-run";
    expect(useTurnStore.getState().start(sessionId, "hello from composer")).toBe(true);
    useTurnStore.getState().onTurnStarted({
      session_id: sessionId,
      turn_id: "t-human",
      input: "hello from composer",
      step_max: 5,
    });
    const rows = useMessageStore.getState().bySession.get(sessionId)?.messages ?? [];
    expect(rows.filter((m) => m.seq < 0)).toHaveLength(0);
    expect(useMessageStore.getState().bySession.get(sessionId)?.pendingUser).toBeTruthy();
  });

  it("onTurnStarted does not treat a compact checkpoint as the last human user", () => {
    const sessionId = "s-cp-last-user";
    useMessageStore.getState().onBufferLoaded(sessionId, {
      session_id: sessionId,
      from_seq: 0,
      to_seq: 1,
      events: [
        {
          seq: 0,
          type: "item/user",
          surface_op: { op: "replace", start: 0, end: 0 },
          item: {
            type: "message",
            role: "user",
            content: [{ type: "input_text", text: "rolled-up" }],
          },
        },
      ],
    });
    useTurnStore.getState().onTurnStarted({
      session_id: sessionId,
      turn_id: "t-after-compact",
      input: "rolled-up",
      step_max: 5,
    });
    const rows = useMessageStore.getState().bySession.get(sessionId)?.messages ?? [];
    const pending = useMessageStore.getState().bySession.get(sessionId)?.pendingUser;
    expect(rows).toHaveLength(1);
    expect(isCompactCutRow(rows[0]!)).toBe(true);
    expect(pending).toBeTruthy();
    expect(itemPlainText(pending!.item)).toBe("rolled-up");
  });
});
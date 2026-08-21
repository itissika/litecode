import { beforeEach, describe, expect, it, vi } from "vitest";

import type { SessionInfo, SessionSnapshot } from "../api/types";
import { useConnectionStore } from "./connectionStore";
import { useMessageStore } from "./messageStore";
import { useSessionStore } from "./sessionStore";
import { useTurnStore } from "./turnStore";

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

describe("sessionStore lifecycle turn_finished", () => {
  beforeEach(() => {
    useTurnStore.setState({ byId: new Map() });
    useSessionStore.setState({
      sessions: [
        {
          id: "s1",
          project: "/p",
          updated_at: 1,
          preview: "",
          running: true,
          turn: {
            turn_id: "t1",
            phase: "calling_llm",
            step: 1,
            step_max: 5,
            started_at_ms: 1,
          },
          agent_id: "default",
          model_id: null,
          api_model_id: "m",
          step_kinds: [],
        } satisfies SessionInfo,
      ],
    } as never);

    useTurnStore.getState().applySnapshotTurn("s1", {
      turn_id: "t1",
      phase: "calling_llm",
      step: 1,
      step_max: 5,
      started_at_ms: 1,
    });
  });

  it("turn_finished forces turnStore idle and clears list running", () => {
    useSessionStore.getState().onSessionLifecycle({
      session_id: "s1",
      event: "turn_finished",
      turn: null,
    });

    expect(useSessionStore.getState().sessions[0].running).toBe(false);
    expect(useSessionStore.getState().sessions[0].turn).toBeNull();
    expect(useTurnStore.getState().byId.get("s1")!.runState).toBe("idle");
    expect(useTurnStore.getState().byId.get("s1")!.currentTurnId).toBeNull();
  });

  it("turn_finished does not idle an optimistic next start", () => {
    useTurnStore.setState({
      byId: new Map([
        [
          "s1",
          {
            ...useTurnStore.getState().byId.get("s1")!,
            runState: "running",
            currentTurnId: null,
          },
        ],
      ]),
    });
    useSessionStore.getState().onSessionLifecycle({
      session_id: "s1",
      event: "turn_finished",
      turn: {
        turn_id: "t1",
        phase: "idle",
        step: 1,
        step_max: 5,
        started_at_ms: 1,
      },
    });
    expect(useTurnStore.getState().byId.get("s1")!.runState).toBe("running");
    expect(useTurnStore.getState().byId.get("s1")!.currentTurnId).toBeNull();
    expect(useSessionStore.getState().sessions[0].running).toBe(false);
  });
});

describe("sessionStore applySnapshot transcript hydrate", () => {
  function snap(
    sessionId: string,
    len: number,
    turn: SessionSnapshot["turn"] = null,
  ): SessionSnapshot {
    return {
      session_id: sessionId,
      project: "/p",
      agent_id: "default",
      api_model_id: "m",
      buffer: { last_seq: len - 1, next_seq: len, revision: 1 },
      turn,
      context_window: 0,
    };
  }

  beforeEach(() => {
    useTurnStore.setState({ byId: new Map() });
    useMessageStore.setState({ bySession: new Map() });
    useConnectionStore.setState({
      sendRpc: vi.fn(async () => ({
        session_id: "s-snap",
        from_seq: 0,
        to_seq: 0,
        events: [],
      })),
    } as never);
  });

  it("cold-opens a history session with buffer/load of the last 40", () => {
    const sendRpc = vi.fn(async () => ({
      session_id: "s-cold",
      from_seq: 0,
      to_seq: 3,
      events: [],
    }));
    useConnectionStore.setState({ sendRpc } as never);
    useSessionStore.getState().applySnapshot(snap("s-cold", 3));
    expect(sendRpc).toHaveBeenCalledWith("buffer/load", {
      from_seq: 0,
      to_seq: 3,
      session_id: "s-cold",
    });
  });

  it("does not reload last-40 after compact when a window already exists", () => {
    const sid = "s-compact-snap";
    const sendRpc = vi.fn();
    useConnectionStore.setState({ sendRpc } as never);
    useMessageStore.getState().onBufferLoaded(sid, {
      session_id: sid,
      from_seq: 0,
      to_seq: 3,
      events: [
        { seq: 0, kind: "item/user", body: { type: "message", role: "user", content: [{ type: "input_text", text: "a" }] } },
        { seq: 1, kind: "item/assistant", body: { type: "message", role: "assistant", id: "a0", status: "completed", content: [{ type: "output_text", text: "b", annotations: [] }] } },
        { seq: 2, kind: "item/user", body: { type: "message", role: "user", content: [{ type: "input_text", text: "c" }] } },
      ],
    });
    useTurnStore.getState().onTurnStarted({
      session_id: sid,
      turn_id: "t-next",
      input: "nightly",
      step_max: 8,
    });

    useSessionStore.getState().applySnapshot({
      ...snap(sid, 4),
      turn: null,
      compacting: false,
      last_turn_token_stats: null,
    });

    expect(sendRpc).not.toHaveBeenCalled();
    const turn = useTurnStore.getState().byId.get(sid)!;
    expect(turn.runState).toBe("running");
    expect(turn.currentTurnId).toBe("t-next");
  });

  it("turn_finished still forces idle after a compact-style snapshot", () => {
    const sid = "s-finish";
    useTurnStore.getState().applySnapshotTurn(sid, {
      turn_id: "t1",
      phase: "calling_llm",
      step: 1,
      step_max: 5,
      started_at_ms: 1,
    });
    useSessionStore.getState().applySnapshot({ ...snap(sid, 2), turn: null });
    expect(useTurnStore.getState().byId.get(sid)!.runState).toBe("running");

    useTurnStore.getState().applySnapshotTurn(sid, null);
    expect(useTurnStore.getState().byId.get(sid)!.runState).toBe("idle");
    expect(useTurnStore.getState().byId.get(sid)!.currentTurnId).toBeNull();
  });
});

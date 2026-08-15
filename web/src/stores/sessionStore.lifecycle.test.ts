import { beforeEach, describe, expect, it, vi } from "vitest";

import type { SessionInfo } from "../api/types";
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
});

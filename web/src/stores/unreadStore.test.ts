import { afterEach, beforeEach, describe, expect, it } from "vitest";

import type { SessionInfo } from "../api/types";
import { bindDockview } from "../dockview/workbench/host";
import { useSessionStore } from "./sessionStore";
import { noteSessionFinished, useUnreadStore } from "./unreadStore";

function session(patch: Partial<SessionInfo> = {}): SessionInfo {
  return {
    id: "s1",
    project: "/p",
    updated_at: 1,
    preview: "one",
    running: false,
    turn: null,
    agent_id: "default",
    api_model_id: "m",
    ...patch,
  };
}

beforeEach(() => {
  useUnreadStore.setState({ unread: {} });
  useSessionStore.setState({ sessions: [] } as never);
});

afterEach(() => {
  bindDockview(null);
});

describe("unreadStore", () => {
  it("marks and consumes idempotently", () => {
    useUnreadStore.getState().mark("s1");
    useUnreadStore.getState().mark("s1");
    expect(useUnreadStore.getState().unread).toEqual({ s1: true });

    useUnreadStore.getState().consume("s1");
    useUnreadStore.getState().consume("s1");
    expect(useUnreadStore.getState().unread).toEqual({});
  });

  it("only marks a finish while the transcript is not visible", () => {
    noteSessionFinished("s1");
    expect(useUnreadStore.getState().unread).toEqual({ s1: true });

    useUnreadStore.setState({ unread: {} });
    bindDockview({
      getPanel: () => ({ api: { isVisible: false } }),
    } as never);
    noteSessionFinished("s1");
    expect(useUnreadStore.getState().unread).toEqual({ s1: true });

    useUnreadStore.setState({ unread: {} });
    bindDockview({
      getPanel: () => ({ api: { isVisible: true } }),
    } as never);
    noteSessionFinished("s1");
    expect(useUnreadStore.getState().unread).toEqual({});

    // A root opened before its classification arrived may live in the
    // read-only subagent host; that transcript still counts as seen.
    useUnreadStore.setState({ unread: {} });
    bindDockview({
      getPanel: (id: string) =>
        id === "subagent-s1" ? { api: { isVisible: true } } : undefined,
    } as never);
    noteSessionFinished("s1");
    expect(useUnreadStore.getState().unread).toEqual({});
  });
});

describe("unreadStore — lifecycle", () => {
  it("marks a root that settles from running to idle", () => {
    useSessionStore.setState({
      sessions: [session({ running: true, status: "running" })],
    } as never);

    useSessionStore.getState().onSessionLifecycle({
      session_id: "s1",
      event: "turn_finished",
      turn: null,
      status: "idle",
    });

    expect(useUnreadStore.getState().unread).toEqual({ s1: true });
  });

  it("waits for a subagent run to settle before marking", () => {
    useSessionStore.setState({
      sessions: [session({ running: true, status: "running_with_subagent" })],
    } as never);

    useSessionStore.getState().onSessionLifecycle({
      session_id: "s1",
      event: "turn_finished",
      turn: null,
      status: "running_with_subagent",
    });
    expect(useUnreadStore.getState().unread).toEqual({});

    useSessionStore.getState().onSessionLifecycle({
      session_id: "s1",
      event: "status_changed",
      turn: null,
      status: "idle",
    });
    expect(useUnreadStore.getState().unread).toEqual({ s1: true });
  });

  it("never marks a subagent child", () => {
    useSessionStore.setState({
      sessions: [
        session({
          running: true,
          parent_session_id: "root",
          parent_call_id: "call_a",
        }),
      ],
    } as never);

    useSessionStore.getState().onSessionLifecycle({
      session_id: "s1",
      event: "turn_finished",
      turn: null,
      status: "idle",
    });

    expect(useUnreadStore.getState().unread).toEqual({});
  });

  it("does not re-mark after a replayed idle update", () => {
    useSessionStore.setState({
      sessions: [session({ running: true, status: "running" })],
    } as never);
    useSessionStore.getState().onSessionLifecycle({
      session_id: "s1",
      event: "turn_finished",
      turn: null,
      status: "idle",
    });
    useUnreadStore.getState().consume("s1");

    useSessionStore.getState().onSessionLifecycle({
      session_id: "s1",
      event: "status_changed",
      turn: null,
      status: "idle",
    });

    expect(useUnreadStore.getState().unread).toEqual({});
  });
});

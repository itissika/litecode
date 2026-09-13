/**
 * Sidebar session list. The server lists subagent CHILD sessions too (the roster
 * needs them to label a never-subscribed child), so the sidebar must show roots
 * only — otherwise every launched worker appears as a session of its own.
 */
import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { SessionInfo } from "../api/types";
import { useConnectionStore } from "../stores/connectionStore";
import { useSessionStore } from "../stores/sessionStore";
import { SessionList } from "./SessionList";

function session(id: string, patch: Partial<SessionInfo> = {}): SessionInfo {
  return {
    id,
    project: "/p",
    updated_at: 0,
    preview: id,
    running: false,
    turn: null,
    agent_id: "default",
    api_model_id: "m",
    ...patch,
  };
}

beforeEach(() => {
  useSessionStore.setState({
    sessions: [],
    sessionsLoading: false,
    sessionListError: null,
  } as never);
});

afterEach(() => {
  cleanup();
  useSessionStore.setState({
    sessions: [],
    sessionsLoading: false,
    sessionListError: null,
  } as never);
  useConnectionStore.setState({ state: "disconnected" } as never);
  vi.restoreAllMocks();
});

describe("SessionList — root filtering (P7)", () => {
  it("renders roots only while the store still carries the children", async () => {
    const root = session("root-1", { preview: "root work" });
    const child = session("child-1", {
      preview: "child work",
      parent_session_id: "root-1",
      parent_call_id: "call_a",
    });
    useConnectionStore.setState({
      state: "connected",
      sendRpc: vi.fn(async () => ({ sessions: [root, child] })),
    } as never);

    render(<SessionList />);

    expect(await screen.findByText("root work")).toBeTruthy();
    // The child is in the store — the roster reads it from there — but the
    // sidebar must not surface it as a session of its own.
    expect(useSessionStore.getState().sessions).toHaveLength(2);
    expect(screen.queryByText("child work")).toBeNull();
    expect(screen.getAllByRole("button", { name: "Delete session" })).toHaveLength(
      1,
    );
    expect(screen.getByText("1 session")).toBeTruthy();
  });

  it("never lists a child that arrived on the lifecycle feed first", async () => {
    const root = session("root-1", { preview: "root work" });
    useConnectionStore.setState({
      state: "connected",
      sendRpc: vi.fn(async () => ({ sessions: [root] })),
    } as never);

    render(<SessionList />);
    expect(await screen.findByText("root work")).toBeTruthy();

    // A child's `created` broadcast can beat the (racy) list push to the client.
    act(() => {
      useSessionStore.getState().onSessionLifecycle({
        session_id: "child-1",
        event: "created",
        project: "/p",
        agent_id: "researcher",
        parent_session_id: "root-1",
        parent_call_id: "call_a",
        updated_at: 2,
        turn: null,
      });
    });

    expect(
      useSessionStore
        .getState()
        .sessions.map((s) => s.id)
        .sort(),
    ).toEqual(["child-1", "root-1"]);
    expect(
      screen.getAllByRole("button", { name: "Delete session" }),
    ).toHaveLength(1);
  });

  it("still shows an empty sidebar when the list carries children only", async () => {
    const child = session("child-1", {
      preview: "child work",
      parent_session_id: "root-1",
    });
    useConnectionStore.setState({
      state: "connected",
      sendRpc: vi.fn(async () => ({ sessions: [child] })),
    } as never);

    render(<SessionList />);

    expect(
      await screen.findByText("No saved sessions yet."),
    ).toBeTruthy();
  });
});

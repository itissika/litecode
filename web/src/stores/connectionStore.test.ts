import { afterEach, describe, expect, it, vi } from "vitest";

import {
  useConnectionStore,
  sessionTitleBot,
} from "./connectionStore";
import { useMessageStore } from "./messageStore";
import { useSessionStore } from "./sessionStore";
import { useTurnStore } from "./turnStore";
import { useGitStore } from "./gitStore";
import { useWorkspaceChangeStore } from "./workspaceChangeStore";

describe("sessionTitleBot", () => {
  const directory = { name: "Stale directory" };

  it("treats bot:null as explicit no-bot, not a directory fallback", () => {
    expect(sessionTitleBot({ bot: null }, directory)).toBeNull();
  });

  it("falls back only when the field is missing", () => {
    expect(sessionTitleBot({}, directory)).toBe(directory);
    expect(sessionTitleBot(undefined, directory)).toBe(directory);
  });

  it("prefers the session bot object", () => {
    const bot = { id: "b1", name: "From session" };
    expect(sessionTitleBot({ bot }, directory)).toBe(bot);
  });
});

describe("agent/subagent_bound → session/list refresh", () => {
  afterEach(() => {
    vi.restoreAllMocks();
    useMessageStore.setState({ bySession: new Map() });
  });

  it("re-pulls the list so a newborn child is known without a subscribe", () => {
    const listSessions = vi
      .spyOn(useSessionStore.getState(), "listSessions")
      .mockImplementation(() => {});

    useConnectionStore.getState().dispatchEnvelope({
      method: "agent/subagent_bound",
      params: {
        session_id: "parent-1",
        call_id: "call_a",
        child_session_id: "child-1",
      },
    });

    expect(listSessions).toHaveBeenCalledTimes(1);
  });

  it("leaves the list alone for unrelated notifications", () => {
    const listSessions = vi
      .spyOn(useSessionStore.getState(), "listSessions")
      .mockImplementation(() => {});

    useConnectionStore.getState().dispatchEnvelope({
      method: "agent/turn_started",
      params: { session_id: "parent-1" },
    });

    expect(listSessions).not.toHaveBeenCalled();
  });
});

describe("workspace/changed → workspace change store", () => {
  afterEach(() => {
    useWorkspaceChangeStore.setState({ last: null, headSeq: 0 });
  });

  it("records every change for panels that re-read files", () => {
    useConnectionStore.getState().dispatchEnvelope({
      method: "workspace/changed",
      params: {
        paths: [".litecode/plan/calm-river.md"],
        kind: "modified",
      },
    });

    expect(useWorkspaceChangeStore.getState().last).toMatchObject({
      paths: [".litecode/plan/calm-river.md"],
      kind: "modified",
    });
  });

  it("records a HEAD move separately from file changes", () => {
    const scheduleRefresh = vi.fn();
    useGitStore.setState({ scheduleRefresh });
    useConnectionStore.getState().dispatchEnvelope({
      method: "git/head",
      params: {},
    });
    expect(useWorkspaceChangeStore.getState().headSeq).toBe(1);
    expect(useWorkspaceChangeStore.getState().last).toBeNull();
    expect(scheduleRefresh).toHaveBeenCalledWith(undefined, { history: true });
  });

  it("refreshes status without moving HEAD when history is false", () => {
    const scheduleRefresh = vi.fn();
    useGitStore.setState({ scheduleRefresh });
    useConnectionStore.getState().dispatchEnvelope({
      method: "git/head",
      params: { history: false },
    });
    expect(useWorkspaceChangeStore.getState().headSeq).toBe(0);
    expect(scheduleRefresh).toHaveBeenCalledWith(undefined, {
      history: false,
    });
  });
});

describe("session/pending_messages → turn store", () => {
  afterEach(() => {
    useTurnStore.setState({ byId: new Map() });
  });

  it("overwrites the queue with the full authority list (absent = empty)", () => {
    useConnectionStore.getState().dispatchEnvelope({
      method: "session/pending_messages",
      params: {
        session_id: "s-pending",
        pending_messages: [
          { id: "p1", text: "first" },
          { id: "p2", text: "second" },
        ],
      },
    });
    expect(
      useTurnStore.getState().byId.get("s-pending")?.pendingMessages,
    ).toEqual([
      { id: "p1", text: "first" },
      { id: "p2", text: "second" },
    ]);

    // The server is the only authority: a later frame with no list empties it.
    useConnectionStore.getState().dispatchEnvelope({
      method: "session/pending_messages",
      params: { session_id: "s-pending" },
    });
    expect(
      useTurnStore.getState().byId.get("s-pending")?.pendingMessages,
    ).toEqual([]);
  });
});

describe("agents/changed → session store", () => {
  afterEach(() => {
    useSessionStore.setState({ primaryAgents: [] });
  });

  it("replaces the picker list with the re-projected primary agents", () => {
    useSessionStore.setState({
      primaryAgents: [{ id: "default", description: "" }],
    });

    useConnectionStore.getState().dispatchEnvelope({
      method: "agents/changed",
      params: {
        agents: [
          { id: "default", description: "General" },
          { id: "reviewer", description: "Reviewer" },
        ],
      },
    });

    // Authoritative list, not a merge: an agent removed in Settings is gone.
    expect(useSessionStore.getState().primaryAgents).toEqual([
      { id: "default", description: "General" },
      { id: "reviewer", description: "Reviewer" },
    ]);
  });

  it("empties the list when the frame carries none", () => {
    useSessionStore.setState({
      primaryAgents: [{ id: "gone", description: "" }],
    });

    useConnectionStore.getState().dispatchEnvelope({
      method: "agents/changed",
      params: {},
    });

    expect(useSessionStore.getState().primaryAgents).toEqual([]);
  });
});

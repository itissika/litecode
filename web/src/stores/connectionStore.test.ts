import { afterEach, describe, expect, it, vi } from "vitest";

import { useConnectionStore, shouldIgnoreForwardedSubagentEvent } from "./connectionStore";
import { useMessageStore } from "./messageStore";
import { useSessionStore } from "./sessionStore";
import { useWorkspaceChangeStore } from "./workspaceChangeStore";

describe("shouldIgnoreForwardedSubagentEvent", () => {
  it("ignores turn/buffer/permission when parent_session_id is set", () => {
    const params = { parent_session_id: "parent-1", session_id: "parent-1" };
    expect(shouldIgnoreForwardedSubagentEvent("agent/turn_started", params)).toBe(
      true,
    );
    expect(shouldIgnoreForwardedSubagentEvent("agent/turn_event", params)).toBe(
      true,
    );
    expect(shouldIgnoreForwardedSubagentEvent("buffer/item", params)).toBe(true);
    expect(
      shouldIgnoreForwardedSubagentEvent("agent/permission_request", params),
    ).toBe(true);
  });

  it("does not ignore unrelated methods or untagged events", () => {
    expect(
      shouldIgnoreForwardedSubagentEvent("session/lifecycle", {
        parent_session_id: "parent-1",
      }),
    ).toBe(false);
    expect(
      shouldIgnoreForwardedSubagentEvent("agent/turn_started", {
        session_id: "s1",
      }),
    ).toBe(false);
    expect(shouldIgnoreForwardedSubagentEvent("agent/turn_started", undefined)).toBe(
      false,
    );
    expect(
      shouldIgnoreForwardedSubagentEvent("agent/subagent_bound", {
        parent_session_id: "parent-1",
        call_id: "c1",
        child_session_id: "child-1",
      }),
    ).toBe(false);
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
    useWorkspaceChangeStore.setState({ last: null });
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
});

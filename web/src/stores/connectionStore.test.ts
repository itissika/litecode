import { describe, expect, it } from "vitest";

import { shouldIgnoreForwardedSubagentEvent } from "./connectionStore";

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

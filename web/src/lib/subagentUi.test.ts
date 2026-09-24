import { describe, expect, it } from "vitest";

import type { SessionInfo } from "../api/types";
import {
  childStatusWord,
  listStatusWord,
  sendStatusWord,
  stopStatusWord,
  waitSettledLine,
  waitTargetCount,
} from "./subagentUi";

function child(patch: Partial<SessionInfo>): SessionInfo {
  return {
    id: "c1",
    project: "/p",
    updated_at: 0,
    preview: "",
    running: false,
    turn: null,
    agent_id: "reviewer",
    api_model_id: "m",
    ...patch,
  };
}

describe("childStatusWord", () => {
  it("prefers live Session over the sealed tool call", () => {
    expect(childStatusWord(undefined, "failed")).toBe("failed");
    expect(childStatusWord(child({ status: "stopping" }), "ok")).toBe(
      "stopping",
    );
    expect(
      childStatusWord(child({ running: true, status: "running" }), "ok"),
    ).toBe("running");
    expect(childStatusWord(child({ status: "idle" }), "ok")).toBe("idle");
    expect(
      childStatusWord(
        child({ status: "idle", last_turn_reason: "cancelled" }),
        "ok",
      ),
    ).toBe("cancelled");
    expect(
      childStatusWord(
        child({ status: "idle", last_turn_reason: "completed" }),
        "ok",
      ),
    ).toBe("completed");
    expect(
      childStatusWord(
        child({ status: "idle", last_turn_reason: "error" }),
        "ok",
      ),
    ).toBe("error");
    expect(childStatusWord(undefined, "running")).toBe("running");
    expect(childStatusWord(undefined, "ok")).toBe("accepted");
  });
});

describe("sendStatusWord", () => {
  it("uses the child when known, else the started-status line", () => {
    expect(sendStatusWord(undefined, "failed")).toBe("failed");
    expect(
      sendStatusWord(child({ running: true, status: "running" }), "ok"),
    ).toBe("running");
    expect(sendStatusWord(undefined, "ok", "status: running\n")).toBe(
      "running",
    );
    expect(sendStatusWord(undefined, "running")).toBe("sending");
    expect(sendStatusWord(undefined, "ok")).toBe("sent");
  });
});

describe("waitSettledLine", () => {
  it("reads the barrier snapshot, not the full reports", () => {
    expect(waitSettledLine("status: nothing to wait for\n")).toBe(
      "nothing to wait",
    );
    expect(
      waitSettledLine(
        "status: settled\nsettled: 1\n---\nchild_session_id: c\nreason: cancelled\nagent: reviewer\n",
      ),
    ).toBe("settled 1 · reviewer · cancelled");
    expect(waitSettledLine("status: settled\nsettled: 2\n")).toBe("settled 2");
    expect(
      waitSettledLine(
        "status: settled\nsettled: 1\n---\nreason: completed\nagent: reviewer\n",
      ),
    ).toBe("settled 1 · reviewer · completed");
  });
});

describe("waitTargetCount", () => {
  it("prefers count, then ids length", () => {
    expect(waitTargetCount({ count: 2, ids: ["a", "b", "c"] })).toBe(2);
    expect(waitTargetCount({ ids: ["a", "b"] })).toBe(2);
    expect(waitTargetCount({})).toBeUndefined();
  });
});

describe("stopStatusWord", () => {
  it("distinguishes request, already ended, and idle", () => {
    expect(stopStatusWord(undefined, "failed")).toBe("failed");
    expect(stopStatusWord(undefined, "running")).toBe("stopping");
    expect(
      stopStatusWord("status: stop_requested\nchild_session_id: c\n", "ok"),
    ).toBe("stop requested");
    expect(
      stopStatusWord("status: already ended\nreason: cancelled\n", "ok"),
    ).toBe("already ended · cancelled");
    expect(stopStatusWord("status: idle\nchild_session_id: c\n", "ok")).toBe(
      "idle",
    );
  });
});

describe("listStatusWord", () => {
  it("shows the session count from the first output line", () => {
    expect(listStatusWord(undefined, "running")).toBe("listing…");
    expect(listStatusWord("sessions: 0\n", "ok")).toBe("none");
    expect(listStatusWord("sessions: 1\n- c  a\n", "ok")).toBe("1 session");
    expect(listStatusWord("sessions: 3\n", "ok")).toBe("3 sessions");
    expect(listStatusWord(undefined, "failed")).toBe("failed");
  });
});

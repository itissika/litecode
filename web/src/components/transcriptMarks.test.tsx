import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import {
  PlanExecuteMark,
  jobExitDetail,
  readableCompactSummary,
  subagentExitDetail,
} from "./transcriptMarks";

afterEach(cleanup);

describe("PlanExecuteMark", () => {
  it("names the plan file the button launched", () => {
    render(<PlanExecuteMark planPath=".litecode/plan/calm.md" />);
    expect(screen.getByTestId("plan-execute-mark").textContent).toBe(
      ".litecode/plan/calm.md 开始执行",
    );
  });

  it("degrades to the bare label with no active plan pointer", () => {
    render(<PlanExecuteMark planPath={null} />);
    expect(screen.getByTestId("plan-execute-mark").textContent).toBe(
      "开始执行",
    );
  });
});

describe("readableCompactSummary", () => {
  it("strips the conversation summary label prefix", () => {
    expect(readableCompactSummary("[Conversation summary]\nDone X and Y")).toBe(
      "Done X and Y",
    );
  });

  it("strips the aggressive summary label prefix", () => {
    expect(readableCompactSummary("[Aggressive summary]\nOnly key facts")).toBe(
      "Only key facts",
    );
  });

  it("removes internal system-reminder blocks", () => {
    const raw =
      "[Conversation summary]\n<system-reminder>\nkeep recent tool results\n</system-reminder>\nProse body";
    expect(readableCompactSummary(raw)).toBe("Prose body");
  });

  it("returns clean text untouched", () => {
    expect(readableCompactSummary("Plain summary")).toBe("Plain summary");
  });
});

describe("jobExitDetail", () => {
  it("reads the exit line of the background-exit reminder", () => {
    expect(
      jobExitDetail(
        "<system-reminder>\nBackground bash bg_a exited with code 3.\noutput_file: .litecode/bash/bg_a.output\ncommand: sleep 8\n</system-reminder>",
      ),
    ).toBe("bg_a · exit code 3");
  });

  it("reads the user-Kill variant", () => {
    expect(
      jobExitDetail(
        "<system-reminder>\nThe user stopped background bash bg_b (Kill).\nexit_code: 137\noutput_file: .litecode/bash/bg_b.output\n</system-reminder>",
      ),
    ).toBe("bg_b · stopped by user (Kill)");
  });

  it("returns undefined when the body carries no exit line", () => {
    expect(jobExitDetail("plain reminder body")).toBeUndefined();
    expect(jobExitDetail("")).toBeUndefined();
  });
});

describe("subagentExitDetail", () => {
  it("collapses a single child into agent and non-completed reason", () => {
    expect(
      subagentExitDetail(`<system-reminder>
source: subagent
status: settled
settled: 1
---
child_session_id: child-abc
reason: cancelled
agent: reviewer
output:
done
</system-reminder>`),
    ).toEqual({
      detail: "settled · reviewer · cancelled",
      childId: "child-abc",
    });
  });

  it("keeps a completed single child to agent only", () => {
    expect(
      subagentExitDetail(
        "source: subagent\nsettled: 1\nchild_session_id: c1\nreason: completed\nagent: explore\n",
      ),
    ).toEqual({ detail: "settled · explore · completed", childId: "c1" });
  });

  it("summarizes a batch as a count", () => {
    expect(
      subagentExitDetail(
        "settled: 3\nchild_session_id: a\nagent: one\n---\nchild_session_id: b\nagent: two\n",
      ),
    ).toEqual({ detail: "3 settled", childId: "a" });
  });
});

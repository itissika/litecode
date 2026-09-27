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

  it("returns clean text untouched", () => {
    expect(readableCompactSummary("Plain summary")).toBe("Plain summary");
  });
});

describe("jobExitDetail", () => {
  it("reads the exit fields of the background-exit reminder", () => {
    expect(
      jobExitDetail({
        kind: "bash_exit",
        exits: [
          {
            job_id: "bg_a",
            command: "sleep 8",
            exit_code: 3,
            killed: false,
            output_file: ".litecode/bash/bg_a.output",
          },
        ],
        running: [],
        text: "Background bash bg_a exited with code 3.",
      }),
    ).toBe("bg_a · exit code 3");
  });

  it("reads the user-Kill variant", () => {
    expect(
      jobExitDetail({
        kind: "bash_exit",
        exits: [
          {
            job_id: "bg_b",
            command: "sleep",
            exit_code: 137,
            killed: true,
            output_file: ".litecode/bash/bg_b.output",
          },
        ],
        running: [],
        text: "The user stopped background bash bg_b (Kill).",
      }),
    ).toBe("bg_b · stopped by user (Kill)");
  });

  it("returns undefined when the body carries no exits", () => {
    expect(
      jobExitDetail({ kind: "bash_exit", exits: [], running: [], text: "" }),
    ).toBeUndefined();
    expect(jobExitDetail(undefined)).toBeUndefined();
  });
});

describe("subagentExitDetail", () => {
  it("collapses a single child into agent and reason", () => {
    expect(
      subagentExitDetail([
        {
          child_session_id: "child-abc",
          turn_id: "t1",
          agent: "reviewer",
          reason: "cancelled",
        },
      ]),
    ).toEqual({
      detail: "settled · reviewer · cancelled",
      childId: "child-abc",
    });
  });

  it("keeps a completed single child to agent and reason", () => {
    expect(
      subagentExitDetail([
        {
          child_session_id: "c1",
          turn_id: "t1",
          agent: "explore",
          reason: "completed",
        },
      ]),
    ).toEqual({ detail: "settled · explore · completed", childId: "c1" });
  });

  it("summarizes a batch as a count", () => {
    expect(
      subagentExitDetail([
        {
          child_session_id: "a",
          turn_id: "t1",
          agent: "one",
          reason: "completed",
        },
        {
          child_session_id: "b",
          turn_id: "t2",
          agent: "two",
          reason: "completed",
        },
        {
          child_session_id: "c",
          turn_id: "t3",
          agent: "three",
          reason: "completed",
        },
      ]),
    ).toEqual({ detail: "3 settled", childId: "a" });
  });
});

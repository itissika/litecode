import { describe, expect, it } from "vitest";

import {
  deriveUserAnchorK,
  hydrateUserDetailBefore,
  isAssistantMessage,
  isHiddenHumanRow,
  isHumanUserRow,
  isHumanViewKind,
  isTranscriptMarkRow,
  isWellFormedBufferRow,
  latestAssistantText,
  mergeCommittedItem,
  optimisticUserSealText,
  sealMismatchError,
  transcriptMarkKind,
  userTextItem,
} from "./adapter";
import type { HumanRow, Item } from "./types";

const userRow = (seq: number, text: string): HumanRow => ({
  seq,
  kind: "item/user",
  state: "final",
  body: userTextItem(text),
});

const assistantRow = (
  seq: number,
  text: string,
  status = "completed",
): HumanRow => ({
  seq,
  kind: "item/assistant",
  state: "final",
  body: {
    type: "message",
    role: "assistant",
    id: `a${seq}`,
    status,
    content: [{ type: "output_text", text, annotations: [] }],
  },
});

const reasoningRow = (seq: number, text: string): HumanRow => ({
  seq,
  kind: "item/assistant",
  state: "final",
  body: {
    type: "reasoning",
    id: `r${seq}`,
    summary: [{ type: "summary_text", text }],
  },
});

describe("kind-based HumanView rows", () => {
  it("counts only explicit item/user rows; job-exit is not a user anchor", () => {
    const rows: HumanRow[] = [
      userRow(0, "u0"),
      {
        seq: 1,
        kind: "compacted",
        state: "final",
        body: { summary: "hidden", from: 0, to: 1 },
      },
      {
        seq: 2,
        kind: "reminder/job_exit",
        state: "final",
        body: userTextItem("<system-reminder>not inspected</system-reminder>"),
      },
      userRow(3, "u1"),
    ];
    expect(deriveUserAnchorK(rows, 3, 5)).toBe(6);
    expect(isHiddenHumanRow(rows[2]!)).toBe(false);
    expect(isHumanUserRow(rows[2]!)).toBe(false);
    expect(isTranscriptMarkRow(rows[1]!)).toBe(true);
    expect(isTranscriptMarkRow(rows[2]!)).toBe(true);
    expect(isHumanUserRow(rows[0]!)).toBe(true);
  });

  it("does not infer log kind from body", () => {
    expect(isHumanViewKind("future/widget")).toBe(false);
    expect(
      isWellFormedBufferRow({
        seq: 1,
        item: userTextItem("legacy"),
      }),
    ).toBe(false);
    expect(
      isWellFormedBufferRow({
        seq: 1,
        kind: "item/user",
        state: "final",
        body: userTextItem("ok"),
      }),
    ).toBe(true);
  });

  it("shows subagent completion reminders as one-line marks, not hidden rows", () => {
    const subagent: HumanRow = {
      seq: 1,
      kind: "reminder/job_exit",
      state: "final",
      body: userTextItem(
        "<system-reminder>\nsource: subagent\nstatus: settled\n</system-reminder>",
      ),
    };
    const other: HumanRow = {
      seq: 2,
      kind: "reminder/job_exit",
      state: "final",
      body: userTextItem("Background bash bg_1 exited"),
    };
    expect(isHiddenHumanRow(subagent)).toBe(false);
    expect(isTranscriptMarkRow(subagent)).toBe(true);
    expect(transcriptMarkKind(subagent)).toBe("subagent_exit");
    expect(isHiddenHumanRow(other)).toBe(false);
    expect(isTranscriptMarkRow(other)).toBe(true);
    expect(transcriptMarkKind(other)).toBe("job_exit");
  });

  it("shows a plan-review reminder as a plan mark, never a job exit", () => {
    const plan: HumanRow = {
      seq: 3,
      kind: "reminder/plan",
      state: "final",
      body: userTextItem(
        "<system-reminder>\n[Plan updated] .litecode/plan/calm.md changed since you last read it.\n</system-reminder>",
      ),
    };
    expect(isHiddenHumanRow(plan)).toBe(false);
    expect(isHumanUserRow(plan)).toBe(false);
    expect(isTranscriptMarkRow(plan)).toBe(true);
    expect(transcriptMarkKind(plan)).toBe("plan");
  });

  it("shows the plan-execution trigger as its own mark, not a user bubble", () => {
    const exec: HumanRow = {
      seq: 4,
      kind: "plan/execute",
      state: "final",
      body: userTextItem("按当前计划开始执行。"),
    };
    expect(isHiddenHumanRow(exec)).toBe(false);
    expect(isHumanViewKind(exec.kind)).toBe(true);
    expect(isHumanUserRow(exec)).toBe(false);
    expect(isTranscriptMarkRow(exec)).toBe(true);
    expect(transcriptMarkKind(exec)).toBe("plan_execute");
    // It seals the optimistic composer bubble it replaced.
    expect(optimisticUserSealText(exec)).toBe("按当前计划开始执行。");
  });

  it("seals optimistic composer bubbles only for user and plan-execute rows", () => {
    expect(optimisticUserSealText(userRow(1, "hi"))).toBe("hi");
    expect(
      optimisticUserSealText({
        seq: 2,
        kind: "reminder/plan",
        state: "final",
        body: userTextItem("hi"),
      }),
    ).toBeNull();
  });

  it("hydrates userDetailBefore from the server prefix for partial windows", () => {
    expect(hydrateUserDetailBefore(10, 3, 0)).toBe(3);
    expect(hydrateUserDetailBefore(0, 3, 9)).toBe(0);
    expect(hydrateUserDetailBefore(10, undefined, 2)).toBe(2);
  });
});

describe("latestAssistantText", () => {
  it("returns the last non-empty assistant text in seq order", () => {
    const rows: HumanRow[] = [
      assistantRow(0, "first probe"),
      assistantRow(1, "checking the tests"),
      assistantRow(2, "wrapping up"),
    ];
    expect(latestAssistantText(rows)).toBe("wrapping up");
  });

  it("skips reasoning items, user rows, and streaming empty shells", () => {
    const rows: HumanRow[] = [
      userRow(0, "user prompt"),
      reasoningRow(1, "thinking…"),
      assistantRow(2, "", "in_progress"),
      assistantRow(3, "done"),
    ];
    expect(latestAssistantText(rows)).toBe("done");
  });

  it("returns an empty string when there is no non-empty assistant text", () => {
    expect(
      latestAssistantText([userRow(0, "hi"), reasoningRow(1, "hmm")]),
    ).toBe("");
    expect(latestAssistantText([])).toBe("");
  });

  it("only considers trailing text when later assistant messages are empty", () => {
    const rows: HumanRow[] = [
      assistantRow(0, "earlier"),
      assistantRow(1, "", "in_progress"),
    ];
    expect(latestAssistantText(rows)).toBe("earlier");
  });
});

describe("sealMismatchError", () => {
  it("flags type contradiction", () => {
    const live: Item = {
      type: "message",
      role: "assistant",
      id: "msg_1",
      status: "in_progress",
      content: [{ type: "output_text", text: "a", annotations: [] }],
    };
    const committed: Item = {
      type: "reasoning",
      id: "msg_1",
      summary: [],
    };
    expect(sealMismatchError(live, committed)).toMatch(/type=/);
  });

  it("allows same-type seal", () => {
    const live: Item = emptyMsg("msg_1", "partial");
    const committed: Item = emptyMsg("msg_1", "final");
    expect(sealMismatchError(live, committed)).toBeNull();
  });
});

describe("mergeCommittedItem", () => {
  it("stamps status when visible text already matches", () => {
    const content = [
      { type: "output_text" as const, text: "hello", annotations: [] },
    ];
    const liveItem: Item = {
      type: "message",
      role: "assistant",
      id: "msg_1",
      status: "in_progress",
      content,
    };
    const merged = mergeCommittedItem(liveItem, {
      type: "message",
      role: "assistant",
      id: "msg_1",
      status: "completed",
      content: [{ type: "output_text", text: "hello", annotations: [] }],
    });
    expect(merged).toMatchObject({ id: "msg_1", status: "completed" });
    expect(isAssistantMessage(merged) && merged.content).toBe(content);
  });

  it("replaces the item when visible content diverges", () => {
    const liveItem = emptyMsg("msg_1", "partial");
    const committed = emptyMsg("msg_1", "final");
    expect(mergeCommittedItem(liveItem, committed)).toBe(committed);
  });
});

function emptyMsg(id: string, text: string): Item {
  return {
    type: "message",
    role: "assistant",
    id,
    status: "completed",
    content: [{ type: "output_text", text, annotations: [] }],
  };
}

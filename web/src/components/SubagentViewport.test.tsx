import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { userTextItem } from "../api/adapter";
import type { HumanRow } from "../api/types";
import { useMessageStore } from "../stores/messageStore";
import { SubagentViewport } from "./SubagentViewport";

afterEach(() => {
  cleanup();
  useMessageStore.getState().reset("child-a");
});

function seed(rows: HumanRow[]): void {
  useMessageStore.setState((s) => {
    const bySession = new Map(s.bySession);
    bySession.set("child-a", {
      bySeq: new Map(rows.map((r) => [r.seq, r])),
      messages: rows,
      display: rows,
      pendingUser: null,
      fromSeq: 0,
      toSeq: rows.length,
      userDetailBefore: 0,
      loadingHistory: false,
      hydrated: true,
      shapeError: null,
      subagentBindings: {},
      blockLogGrowth: false,
      turnEndNotice: null,
      itemIdToSeq: new Map(),
    });
    return { bySession };
  });
}

const userRow = (seq: number, text: string): HumanRow => ({
  seq,
  kind: "item/user",
  body: userTextItem(text),
});

const assistantRow = (seq: number, text: string): HumanRow => ({
  seq,
  kind: "item/assistant",
  body: {
    type: "message",
    role: "assistant",
    id: `msg-${seq}`,
    status: "completed",
    content: [{ type: "output_text", text, annotations: [] }],
  },
});

const jobExitRow = (seq: number): HumanRow => ({
  seq,
  kind: "reminder/job_exit",
  body: userTextItem("<system-reminder>bash job exited</system-reminder>"),
});

describe("SubagentViewport skipUserText", () => {
  it("suppresses the leading user row that matches the launch prompt", () => {
    seed([userRow(0, "do the thing"), assistantRow(1, "done")]);
    render(<SubagentViewport childSessionId="child-a" skipUserText="do the thing" />);
    expect(screen.queryByText("do the thing")).toBeNull();
    expect(screen.getByText("done")).toBeTruthy();
  });

  it("keeps a later user row even when it matches the prompt (only prefix is stripped)", () => {
    seed([
      userRow(0, "do the thing"),
      assistantRow(1, "done"),
      userRow(2, "do the thing"),
    ]);
    render(<SubagentViewport childSessionId="child-a" skipUserText="do the thing" />);
    expect(screen.getAllByText("do the thing")).toHaveLength(1);
  });

  it("skips leading transcript marks before the matching prompt", () => {
    seed([
      jobExitRow(0),
      userRow(1, "do the thing"),
      assistantRow(2, "done"),
    ]);
    render(<SubagentViewport childSessionId="child-a" skipUserText="do the thing" />);
    expect(screen.queryByText("do the thing")).toBeNull();
    expect(screen.getByText("done")).toBeTruthy();
  });

  it("skips leading control rows (turn/start) before the matching prompt", () => {
    seed([
      { seq: 0, kind: "turn/start", body: { turn: "t1" } },
      userRow(1, "do the thing"),
      assistantRow(2, "done"),
    ]);
    render(<SubagentViewport childSessionId="child-a" skipUserText="do the thing" />);
    expect(screen.queryByText("do the thing")).toBeNull();
    expect(screen.getByText("done")).toBeTruthy();
  });

  it("fails open when the leading user row does not match the prompt", () => {
    seed([userRow(0, "other message"), assistantRow(1, "done")]);
    render(<SubagentViewport childSessionId="child-a" skipUserText="do the thing" />);
    expect(screen.getByText("other message")).toBeTruthy();
    expect(screen.getByText("done")).toBeTruthy();
  });

  it("fails open when the transcript does not start with a user row", () => {
    seed([assistantRow(0, "done")]);
    render(<SubagentViewport childSessionId="child-a" skipUserText="do the thing" />);
    expect(screen.getByText("done")).toBeTruthy();
  });

  it("shows the leading user row when no skipUserText is provided", () => {
    seed([userRow(0, "do the thing"), assistantRow(1, "done")]);
    render(<SubagentViewport childSessionId="child-a" />);
    expect(screen.getByText("do the thing")).toBeTruthy();
    expect(screen.getByText("done")).toBeTruthy();
  });
});

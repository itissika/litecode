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

describe("SubagentViewport renders every child row (no user-message suppression)", () => {
  it("renders the leading user row (the launch prompt) as content", () => {
    seed([userRow(0, "do the thing"), assistantRow(1, "done")]);
    render(<SubagentViewport childSessionId="child-a" />);
    expect(screen.getByText("do the thing")).toBeTruthy();
    expect(screen.getByText("done")).toBeTruthy();
  });

  it("keeps later user rows — e.g. every subagent_send message", () => {
    seed([
      userRow(0, "do the thing"),
      assistantRow(1, "done"),
      userRow(2, "now do this"),
      assistantRow(3, "ok"),
    ]);
    render(<SubagentViewport childSessionId="child-a" />);
    expect(screen.getByText("do the thing")).toBeTruthy();
    expect(screen.getByText("now do this")).toBeTruthy();
  });

  it("keeps a repeated user message (same text twice)", () => {
    seed([
      userRow(0, "do the thing"),
      assistantRow(1, "done"),
      userRow(2, "do the thing"),
    ]);
    render(<SubagentViewport childSessionId="child-a" />);
    expect(screen.getAllByText("do the thing")).toHaveLength(2);
  });

  it("shows the empty placeholder for a child with no rows", () => {
    seed([]);
    render(<SubagentViewport childSessionId="child-a" />);
    expect(screen.getByText("Empty subagent session")).toBeTruthy();
  });
});

import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { SubagentToolView } from "./SubagentToolView";
import { useMessageStore } from "../../stores/messageStore";
import { useConnectionStore } from "../../stores/connectionStore";

afterEach(() => {
  cleanup();
  useConnectionStore.setState({ state: "disconnected" });
  useMessageStore.getState().reset("parent");
  useMessageStore.getState().reset("child-a");
});

describe("SubagentToolView", () => {
  it("shows a launching placeholder and the task brief before binding", () => {
    render(
      <SubagentToolView
        name="subagent_launch"
        status="running"
        input={{ agent: "worker", prompt: "do the thing" }}
        call_id="call_a"
        sessionId="parent"
      />,
    );
    expect(screen.getByText("Launching subagent…")).toBeTruthy();
    // Input brief (the task) is shown in the body even before binding — appears
    // as the Task FoldCard header preview (always mounted) and, once the card is
    // ready, in its body. Use getAllByText to tolerate both.
    expect(screen.getAllByText("do the thing").length).toBeGreaterThan(0);
  });

  it("is purely presentational: never subscribes — the owning ToolCallCard does", () => {
    useConnectionStore.setState({ state: "connected" });
    useMessageStore.getState().onSubagentBound("parent", {
      session_id: "parent",
      call_id: "call_a",
      child_session_id: "child-a",
    });
    const spy = vi.spyOn(useConnectionStore.getState(), "ensureSubscribe");

    render(
      <SubagentToolView
        name="subagent_launch"
        status="running"
        input={{ agent: "worker", prompt: "do the thing" }}
        call_id="call_a"
        sessionId="parent"
      />,
    );

    expect(spy).not.toHaveBeenCalled();
    spy.mockRestore();
  });
});

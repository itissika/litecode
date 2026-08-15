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
  it("shows a launching placeholder and does not subscribe before binding", () => {
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
    expect(screen.getByText("Launching subagent…")).toBeTruthy();
    // Input brief (the task) is shown in the body even before binding — appears
    // as the Task FoldCard header preview (always mounted) and, once the card is
    // ready, in its body. Use getAllByText to tolerate both.
    expect(screen.getAllByText("do the thing").length).toBeGreaterThan(0);
    expect(spy).not.toHaveBeenCalled();
    spy.mockRestore();
  });

  it("resolves the child session id and subscribes once bound", () => {
    useConnectionStore.setState({ state: "connected" });
    useMessageStore.getState().onSubagentBound("parent", {
      session_id: "parent",
      call_id: "call_a",
      child_session_id: "child-a",
    });
    const spy = vi
      .spyOn(useConnectionStore.getState(), "ensureSubscribe")
      .mockResolvedValue(undefined);

    render(
      <SubagentToolView
        name="subagent_launch"
        status="running"
        input={{ agent: "worker", prompt: "do the thing" }}
        call_id="call_a"
        sessionId="parent"
      />,
    );

    expect(spy).toHaveBeenCalledWith("child-a");
    // Body renders the Task brief (the agent name now lives in the outer
    // tool FoldCard header, not here).
    expect(screen.getAllByText("do the thing").length).toBeGreaterThan(0);
    spy.mockRestore();
  });

  it("unsubscribes and clears the child slice on unmount", () => {
    useConnectionStore.setState({ state: "connected" });
    useMessageStore.getState().onSubagentBound("parent", {
      session_id: "parent",
      call_id: "call_a",
      child_session_id: "child-a",
    });
    const spy = vi
      .spyOn(useConnectionStore.getState(), "ensureSubscribe")
      .mockResolvedValue(undefined);

    const { unmount } = render(
      <SubagentToolView
        name="subagent_launch"
        status="running"
        input={{ agent: "worker", prompt: "do the thing" }}
        call_id="call_a"
        sessionId="parent"
      />,
    );
    expect(spy).toHaveBeenCalledWith("child-a");

    const unsub = vi.spyOn(useConnectionStore.getState(), "unsubscribeSession");
    unmount();
    expect(unsub).toHaveBeenCalledWith("child-a");
    unsub.mockRestore();
    spy.mockRestore();
  });
});

import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { KillShellToolView } from "./KillShellToolView";
import { WaitShellToolView } from "./WaitShellToolView";
import { useBashStore } from "../../stores/bashStore";
import { formatElapsed } from "../../lib/bashLive";

afterEach(() => {
  cleanup();
  useBashStore.getState().reset();
});

describe("WaitShellToolView", () => {
  it("keys countdown elapsed by this call_id's waiter", () => {
    const now = Date.now();
    useBashStore.getState().applySnapshot("s1", {
      jobs: [],
      waits: [
        {
          call_id: "wait_a",
          watching_id: "bg_a",
          started_at_ms: now,
          deadline_ms: now + 12_000,
        },
        {
          call_id: "wait_b",
          watching_id: "bg_b",
          started_at_ms: now,
          deadline_ms: now + 90_000,
        },
      ],
    });

    render(
      <WaitShellToolView
        name="wait_shell"
        status="running"
        input={{ id: "bg_a", sec: 12 }}
        call_id="wait_a"
        sessionId="s1"
      />,
    );

    const label = screen.getByTestId("wait-elapsed").textContent ?? "";
    expect(label).toMatch(/wait\s+1[12]s/);
    expect(label).not.toContain(formatElapsed(90_000));
    expect(document.querySelectorAll(".wait-wave-char").length).toBeGreaterThan(0);
  });

  it("waves while waiting before the waiter snapshot arrives", () => {
    render(
      <WaitShellToolView
        name="wait_shell"
        status="running"
        input={{ id: "bg_a", sec: 12 }}
        call_id="wait_a"
        sessionId="s1"
      />,
    );
    expect(screen.getByTestId("wait-pending")).toBeTruthy();
    expect(document.querySelectorAll(".wait-wave-char").length).toBeGreaterThan(0);
  });

  it("shows waited after the tool result seals and the waiter is gone", () => {
    render(
      <WaitShellToolView
        name="wait_shell"
        status="ok"
        input={{ id: "bg_a", sec: 12 }}
        output={{
          type: "function_call_output",
          call_id: "wait_a",
          output: "status: waited\nrunning: 1\n",
        }}
        call_id="wait_a"
        sessionId="s1"
      />,
    );
    expect(screen.getByText("waited")).toBeTruthy();
    expect(screen.queryByText(/running: 1/)).toBeNull();
    expect(document.querySelector(".wait-wave-char")).toBeNull();
  });
});

describe("KillShellToolView", () => {
  it("shows which bash_id was killed", () => {
    render(
      <KillShellToolView
        name="kill_shell"
        status="ok"
        input={{ bash_id: "bg_a" }}
        output={{
          type: "function_call_output",
          call_id: "kill_1",
          output: "Terminated background task 'bg_a' (exit_code: -1).\n",
        }}
      />,
    );
    expect(screen.getByText("killed bg_a")).toBeTruthy();
    expect(screen.queryByText(/Terminated/)).toBeNull();
  });
});

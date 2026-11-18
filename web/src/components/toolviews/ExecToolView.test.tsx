import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { useBashStore } from "../../stores/bashStore";
import { useConnectionStore } from "../../stores/connectionStore";
import { ExecToolView } from "./ExecToolView";
import { LiveTerminalView } from "./LiveTerminalView";
import { getToolView } from "./registry";

afterEach(() => {
  cleanup();
});

beforeEach(() => {
  useBashStore.getState().reset();
  useConnectionStore.setState({ state: "disconnected", sendRpc: null } as never);
});

describe("ExecToolView", () => {
  it("registers views for exec_command and write_stdin", () => {
    expect(getToolView("exec_command")).toBe(ExecToolView);
    expect(getToolView("write_stdin")).toBe(ExecToolView);
  });

  it("renders direct exec_command cmd and output without regex parsing", () => {
    render(
      <ExecToolView
        name="exec_command"
        status="ok"
        input={{ cmd: "echo hello" }}
        output={{
          type: "function_call_output",
          call_id: "c1",
          output: "exit_code: 0\nhello\n",
        }}
        call_id="c1"
      />,
    );
    const console = screen.getByTestId("exec-console");
    expect(console.textContent).toContain("echo hello");
    expect(console.textContent).toContain("exit_code: 0");
    expect(console.textContent).toContain("hello");
  });

  it("renders write_stdin session_id and chars summary", () => {
    render(
      <ExecToolView
        name="write_stdin"
        status="ok"
        input={{ session_id: "bg_7", chars: "yes\n" }}
        output={{
          type: "function_call_output",
          call_id: "c2",
          output: "exit_code: 0\nok\n",
        }}
        call_id="c2"
      />,
    );
    const console = screen.getByTestId("exec-console");
    expect(console.textContent).toContain("bg_7");
    expect(console.textContent).toContain('"yes\\n"');
    expect(console.textContent).toContain("ok");
  });
});

describe("LiveTerminalView", () => {
  it("polls bash/tail and retains output after process exit", async () => {
    vi.useFakeTimers();
    let calls = 0;
    const sendRpc = vi.fn(async (method: string) => {
      if (method !== "bash/tail") throw new Error(method);
      calls += 1;
      if (calls === 1) {
        return {
          text: "tick 1\n",
          truncated_on_disk: false,
          alive: true,
          exit_code: null,
        };
      }
      return {
        text: "tick 1\ndone\n",
        truncated_on_disk: false,
        alive: false,
        exit_code: 0,
      };
    });
    useConnectionStore.setState({ state: "connected", sendRpc } as never);

    render(
      <LiveTerminalView
        job={{
          id: "bg_1",
          call_id: "c1",
          command_preview: "npm test",
          output_file: ".litecode/run/bg_1.stdout",
          started_at_ms: Date.now(),
        }}
        command="npm test -- --watch=false"
        sessionId="s1"
      />,
    );

    await act(async () => {
      await Promise.resolve();
    });
    expect(screen.getByTestId("live-terminal").textContent).toContain("tick 1");

    await act(async () => {
      await vi.advanceTimersByTimeAsync(300);
    });

    expect(screen.getByTestId("live-terminal-footer").textContent).toContain(
      "exit_code: 0",
    );
    expect(screen.getByTestId("live-terminal").textContent).toContain("done");
    expect(screen.getByTestId("live-terminal-footer").textContent).toContain(
      ".litecode/run/bg_1.stdout",
    );
    vi.useRealTimers();
  });
});

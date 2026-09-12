import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import type { FunctionCallItem, FunctionCallOutputItem } from "../api/types";
import { useSessionStore } from "../stores/sessionStore";
import { useBashStore } from "../stores/bashStore";
import { useTurnStore } from "../stores/turnStore";
import { InlineToolRow } from "./InlineToolRow";

function launchCall(args: Record<string, unknown>): FunctionCallItem {
  return {
    type: "function_call",
    id: "fc",
    call_id: "call_a",
    name: "subagent_launch",
    arguments: JSON.stringify(args),
    status: "completed",
  };
}

const output: FunctionCallOutputItem = {
  type: "function_call_output",
  call_id: "call_a",
  output: "done",
};

afterEach(() => {
  cleanup();
  useSessionStore.setState({ sessions: [] });
  useBashStore.getState().reset();
  useTurnStore.setState({ byId: new Map() });
});

describe("InlineToolRow — subagent_launch is a single-line row", () => {
  it("shows agent + running status with no FoldCard", () => {
    const { container } = render(
      <InlineToolRow call={launchCall({ agent: "explore", prompt: "go" })} streaming />,
    );
    const line = screen.getByTestId("subagent-launch-line");
    expect(line.textContent).toContain("explore");
    expect(line.textContent).toContain("running");
    // Single line — the call is NOT a rich tool card.
    expect(container.querySelector(".foldcard-header")).toBeNull();
  });

  it("seals to a completed status once output is present", () => {
    render(
      <InlineToolRow
        call={launchCall({ agent: "worker" })}
        output={output}
        streaming={false}
      />,
    );
    expect(screen.getByTestId("subagent-launch-line").textContent).toContain(
      "completed",
    );
  });

  it("falls back to a generic label when the agent is unknown", () => {
    render(<InlineToolRow call={launchCall({ prompt: "go" })} streaming />);
    expect(screen.getByTestId("subagent-launch-line").textContent).toContain(
      "subagent",
    );
  });
});

function sendCall(args: Record<string, unknown>, status = "completed"): FunctionCallItem {
  return {
    type: "function_call",
    id: "fc",
    call_id: "call_send",
    name: "subagent_send",
    arguments: JSON.stringify(args),
    status,
  };
}

function sendOutput(text: string): FunctionCallOutputItem {
  return { type: "function_call_output", call_id: "call_send", output: text };
}

function seedSession(id: string, agentId: string): void {
  useSessionStore.setState({
    sessions: [
      {
        id,
        project: "/p",
        updated_at: 0,
        preview: "hi",
        running: false,
        turn: null,
        agent_id: agentId,
        api_model_id: "m",
      },
    ],
  });
}

describe("InlineToolRow — subagent_send is a single-line row", () => {
  it("resolves the agent name instead of falling through to the kill view", () => {
    seedSession("child-abcdef1234", "researcher");
    const { container } = render(
      <InlineToolRow call={sendCall({ id: "child-abcdef1234", message: "go" })} />,
    );
    expect(container.textContent).toContain("sent to researcher");
    expect(container.textContent).not.toContain("killed");
  });

  it("falls back to a short child id when the session is unknown", () => {
    const { container } = render(
      <InlineToolRow call={sendCall({ id: "child-abcdef1234" })} />,
    );
    expect(container.textContent).toContain("sent to child-ab");
  });

  it("appends a flattened, truncated reply summary", () => {
    seedSession("child-abcdef1234", "researcher");
    render(
      <InlineToolRow call={sendCall({ id: "child-abcdef1234" })} output={sendOutput(`${'x'.repeat(200)}\n\nsecond`) } />,
    );
    const summary = screen.getByTestId("subagent-send-summary");
    expect(summary.textContent).not.toContain("\n");
    expect(summary.textContent?.endsWith("…")).toBe(true);
    expect(summary.textContent?.length).toBe(80);
  });

  it("shows send failed (no reply summary) when the call failed", () => {
    seedSession("child-abcdef1234", "researcher");
    render(
      <InlineToolRow
        call={sendCall({ id: "child-abcdef1234" }, "failed")}
        output={sendOutput("Error: nope")}
      />,
    );
    expect(screen.getByText("send failed")).toBeTruthy();
    expect(screen.queryByTestId("subagent-send-summary")).toBeNull();
  });
});

function toolCall(
  name: string,
  args: Record<string, unknown>,
  status = "completed",
): FunctionCallItem {
  return {
    type: "function_call",
    id: "fc",
    call_id: "call_t",
    name,
    arguments: JSON.stringify(args),
    status,
  };
}

function toolOutput(text: string): FunctionCallOutputItem {
  return { type: "function_call_output", call_id: "call_t", output: text };
}

const RUNNING_BASH = "status: running\nbash_id: bg_a\noutput_file: .litecode/bash/bg_a.output\n";

function seedBashJob(callId = "call_t", id = "bg_a"): void {
  useBashStore.getState().applySnapshot("s1", {
    jobs: [
      {
        id,
        call_id: callId,
        command_preview: "sleep 8",
        output_file: `.litecode/bash/${id}.output`,
        started_at_ms: Date.now(),
      },
    ],
    waits: [],
  });
}

describe("InlineToolRow — session-mount capsules render as single-line summaries", () => {
  it("renders todo as the summary toolTitle already produces", () => {
    const { container } = render(
      <InlineToolRow
        call={toolCall("todo", { todos: [{ content: "ship", status: "in_progress" }] })}
        output={toolOutput("OK. Status — pending: 2, in_progress: 1, completed: 3")}
      />,
    );
    expect(screen.getByTestId("inline-todo-summary").textContent).toBe(
      "1 active · 2 pending · 3 done",
    );
    expect(container.querySelector(".foldcard-header")).toBeNull();
  });

  it("renders plan as the created plan path", () => {
    render(
      <InlineToolRow
        call={toolCall("plan", { action: "create", content: "# Plan" })}
        output={toolOutput(
          "Created plan at .litecode/plan/calm-river.md\nPlan filename was auto-generated; content saved.",
        )}
      />,
    );
    expect(screen.getByTestId("inline-plan-summary").textContent).toBe(
      ".litecode/plan/calm-river.md",
    );
  });
});

describe("InlineToolRow — background bash is a single-line row", () => {
  it("shows the command, a running timer and the Kill action without a card", () => {
    seedBashJob();
    const { container } = render(
      <InlineToolRow
        call={toolCall("bash", { command: "sleep 8" })}
        output={toolOutput(RUNNING_BASH)}
        sessionId="s1"
      />,
    );

    expect(screen.getByTestId("inline-bash-command").textContent).toBe("sleep 8");
    expect(screen.getByTestId("inline-bash-status").textContent).toMatch(/^\d+s$/);
    expect(screen.getByTestId("inline-bash-kill")).toBeTruthy();
    expect(container.querySelector(".foldcard-header")).toBeNull();
  });

  it("settles to `exited` and drops Kill once the job leaves the snapshot", () => {
    seedBashJob();
    render(
      <InlineToolRow
        call={toolCall("bash", { command: "sleep 8" })}
        output={toolOutput(RUNNING_BASH)}
        sessionId="s1"
      />,
    );
    expect(screen.getByTestId("inline-bash-kill")).toBeTruthy();

    act(() => {
      useBashStore.getState().applySnapshot("s1", { jobs: [], waits: [] });
    });

    expect(screen.getByTestId("inline-bash-status").textContent).toBe("exited");
    expect(screen.queryByTestId("inline-bash-kill")).toBeNull();
  });

  it("reports a seeded exit code instead of the timer", () => {
    render(
      <InlineToolRow
        call={toolCall("bash", { command: "false" })}
        output={toolOutput("exit_code: 1\n")}
        sessionId="s1"
      />,
    );
    expect(screen.getByTestId("inline-bash-status").textContent).toBe("exit_code: 1");
    expect(screen.queryByTestId("inline-bash-kill")).toBeNull();
  });
});

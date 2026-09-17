import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import type { FunctionCallItem, FunctionCallOutputItem, SessionInfo } from "../api/types";
import { useMessageStore } from "../stores/messageStore";
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
  useMessageStore.setState({ bySession: new Map() });
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

  it("seals to accepted when the child Session is not in the list yet", () => {
    render(
      <InlineToolRow
        call={launchCall({ agent: "worker" })}
        output={output}
        streaming={false}
      />,
    );
    expect(screen.getByTestId("subagent-launch-line").textContent).toContain(
      "accepted",
    );
  });

  it("follows the bound child Session once it is idle", () => {
    useMessageStore.getState().onSubagentBound("s1", {
      session_id: "s1",
      call_id: "call_a",
      child_session_id: "child-1",
    });
    seedSession("child-1", "worker", { running: false, status: "idle" });
    render(
      <InlineToolRow
        call={launchCall({ agent: "worker", responsibility: "review the diff" })}
        output={output}
        streaming={false}
        sessionId="s1"
      />,
    );
    const line = screen.getByTestId("subagent-launch-line");
    expect(line.textContent).toContain("worker");
    expect(line.textContent).toContain("review the diff");
    expect(line.textContent).toContain("idle");
  });

  it("shows the bound child's last turn reason after it settles", () => {
    useMessageStore.getState().onSubagentBound("s1", {
      session_id: "s1",
      call_id: "call_a",
      child_session_id: "child-1",
    });
    seedSession("child-1", "worker", {
      running: false,
      status: "idle",
      last_turn_reason: "cancelled",
    });
    render(
      <InlineToolRow
        call={launchCall({ agent: "worker" })}
        output={output}
        streaming={false}
        sessionId="s1"
      />,
    );
    expect(screen.getByTestId("subagent-launch-line").textContent).toContain(
      "cancelled",
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

function seedSession(
  id: string,
  agentId: string,
  patch: Partial<SessionInfo> = {},
): void {
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
        ...patch,
      },
    ],
  });
}

describe("InlineToolRow — subagent_send is a single-line row", () => {
  it("resolves the agent name and live status instead of dumping started output", () => {
    seedSession("child-abcdef1234", "researcher", {
      running: true,
      status: "running",
    });
    const { container } = render(
      <InlineToolRow
        call={sendCall({ id: "child-abcdef1234", message: "keep going" })}
        output={sendOutput("status: running\nchild_session_id: child-abcdef1234\nThe child runs in the background\n")}
      />,
    );
    const line = screen.getByTestId("subagent-send-line");
    expect(line.textContent).toContain("researcher");
    expect(line.textContent).toContain("keep going");
    expect(line.textContent).toContain("running");
    expect(container.textContent).not.toContain("The child runs in the background");
    expect(container.querySelector(".foldcard-header")).toBeNull();
  });

  it("falls back to a short child id when the session is unknown", () => {
    render(
      <InlineToolRow
        call={sendCall({ id: "child-abcdef1234" })}
        output={sendOutput("status: running\n")}
      />,
    );
    expect(screen.getByTestId("subagent-send-line").textContent).toContain("child-ab");
    expect(screen.getByTestId("subagent-send-line").textContent).toContain("running");
  });

  it("shows failed when the call failed", () => {
    seedSession("child-abcdef1234", "researcher");
    render(
      <InlineToolRow
        call={sendCall({ id: "child-abcdef1234" }, "failed")}
        output={sendOutput("Error: nope")}
      />,
    );
    expect(screen.getByTestId("subagent-send-line").textContent).toContain("failed");
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

describe("InlineToolRow — remaining subagent tools are single-line rows", () => {
  it("shows wait target count while pending and settled N after output", () => {
    const { rerender, container } = render(
      <InlineToolRow
        call={toolCall("subagent_wait", { ids: ["a", "b"], count: 2 }, "in_progress")}
        streaming
      />,
    );
    expect(screen.getByTestId("subagent-wait-pending").textContent?.replace(/\u00a0/g, " ")).toContain(
      "waiting 2",
    );
    expect(container.querySelector(".foldcard-header")).toBeNull();

    rerender(
      <InlineToolRow
        call={toolCall("subagent_wait", { ids: ["a", "b"], count: 2 })}
        output={toolOutput(
          "status: settled\nsettled: 1\n---\nchild_session_id: a\nreason: cancelled\nagent: reviewer\n",
        )}
      />,
    );
    expect(screen.getByTestId("subagent-wait-line").textContent).toBe(
      "settled 1 · reviewer · cancelled",
    );
  });

  it("shows stop already-ended from the tool output", () => {
    seedSession("child-abcdef1234", "reviewer");
    render(
      <InlineToolRow
        call={toolCall("subagent_stop", { id: "child-abcdef1234" })}
        output={toolOutput("status: already ended\nreason: completed\n")}
      />,
    );
    expect(screen.getByTestId("subagent-stop-line").textContent).toContain("reviewer");
    expect(screen.getByTestId("subagent-stop-line").textContent).toContain(
      "already ended · completed",
    );
  });

  it("shows list as a session count, not a FoldCard dump", () => {
    const { container } = render(
      <InlineToolRow
        call={toolCall("subagent_list", {})}
        output={toolOutput("sessions: 2\n- a  worker\n- b  explore\n")}
      />,
    );
    expect(screen.getByTestId("subagent-list-line").textContent).toBe("2 sessions");
    expect(container.querySelector(".foldcard-header")).toBeNull();
  });
});

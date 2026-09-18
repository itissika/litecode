import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../api/workspace", () => ({ readFile: vi.fn() }));

import type { BashJob, HumanRow, SessionInfo } from "../api/types";
import { readFile } from "../api/workspace";
import { useBashStore } from "../stores/bashStore";
import { setDockviewApi, useConnectionStore } from "../stores/connectionStore";
import { useEditorStore } from "../stores/editorStore";
import {
  emptySlice as emptyMessageSlice,
  useMessageStore,
} from "../stores/messageStore";
import { useSessionStore } from "../stores/sessionStore";
import { emptySlice, useTurnStore, type TurnSlice } from "../stores/turnStore";
import { useWorkspaceChangeStore } from "../stores/workspaceChangeStore";
import {
  BASH_CLAIM_GRACE_MS,
  PANEL_EXIT_MS,
  PANEL_INITIAL_H,
  PANEL_MAX_H,
  PLAN_EXECUTE_PROMPT,
  SessionStatusLine,
  panelInitialHeight,
} from "./SessionStatusLine";

const bashJob: BashJob = {
  id: "bg_a",
  call_id: "c1",
  command_preview: "sleep 1",
  output_file: ".litecode/bash/bg_a.output",
  started_at_ms: Date.now(),
};

const bashJob2: BashJob = {
  id: "bg_b",
  call_id: "c2",
  command_preview: "sleep 2",
  output_file: ".litecode/bash/bg_b.output",
  started_at_ms: Date.now(),
};

function subagentSession(
  id = "sa_a",
  parent = "s1",
  running = true,
  agent = "researcher",
): SessionInfo {
  return {
    id,
    project: "/p",
    updated_at: Date.now(),
    preview: "find the wiring",
    running,
    status: running ? "running" : "idle",
    turn: running
      ? { turn_id: `turn-${id}`, phase: "calling_llm", step: 1, step_max: 10, started_at_ms: Date.now() }
      : null,
    agent_id: agent,
    api_model_id: "m",
    parent_session_id: parent,
    parent_call_id: `call-${id}`,
    responsibility: "research",
  };
}

function seedTurn(sessionId: string, patch: Partial<TurnSlice>) {
  useTurnStore.setState({
    byId: new Map([[sessionId, { ...emptySlice(), ...patch }]]),
  });
}

/** Transcript rows for a bash call — the capsule's background verdict and the
 *  full command come from these (the job wire only carries a preview). */
function bashCallRow(
  callId: string,
  args: Record<string, unknown>,
  seq = 1,
): HumanRow {
  return {
    seq,
    kind: "item/tool_call",
    streaming: false,
    body: {
      type: "function_call",
      id: `fc_${callId}`,
      call_id: callId,
      name: "bash",
      arguments: JSON.stringify(args),
      status: "completed",
    },
  };
}

function bashResultRow(callId: string, output: string, seq = 2): HumanRow {
  return {
    seq,
    kind: "item/tool_result",
    streaming: false,
    body: { type: "function_call_output", call_id: callId, output },
  };
}

function seedRows(sessionId: string, rows: HumanRow[]) {
  useMessageStore.setState((state) => {
    const bySession = new Map(state.bySession);
    bySession.set(sessionId, {
      ...emptyMessageSlice(),
      messages: rows,
      display: rows,
    });
    return { bySession };
  });
}

const originalOpenFile = useEditorStore.getState().openFile;

beforeEach(() => {
  useBashStore.getState().reset();
  useMessageStore.setState({ bySession: new Map() });
  useTurnStore.setState({ byId: new Map() });
  useSessionStore.setState({ project: null, sessions: [], byId: new Map() } as never);
  useEditorStore.setState({ openFile: originalOpenFile } as never);
  useWorkspaceChangeStore.setState({ last: null });
  vi.mocked(readFile).mockReset().mockResolvedValue("# plan");
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  useBashStore.getState().reset();
  useMessageStore.setState({ bySession: new Map() });
  useTurnStore.setState({ byId: new Map() });
  useSessionStore.setState({ project: null, sessions: [], byId: new Map() } as never);
  useEditorStore.setState({ openFile: originalOpenFile } as never);
  useWorkspaceChangeStore.setState({ last: null });
});

describe("SessionStatusLine — resident capsules", () => {
  it("keeps all four capsules resident with no data (never appear/disappear)", () => {
    render(<SessionStatusLine sessionId="s1" />);
    expect(screen.getByTestId("capsule-terminal")).toBeTruthy();
    expect(screen.getByTestId("capsule-subagent")).toBeTruthy();
    expect(screen.getByTestId("capsule-plan")).toBeTruthy();
    expect(screen.getByTestId("capsule-todo")).toBeTruthy();
    // No panel is open until a capsule is clicked.
    expect(screen.queryByTestId("status-capsule-panel")).toBeNull();
  });

  it("uses the Latest-button glass and never dims (no opacity / rounded-full)", () => {
    render(<SessionStatusLine sessionId="s1" />);
    for (const id of ["terminal", "subagent", "plan", "todo"] as const) {
      const cls = screen.getByTestId(`capsule-${id}`).className;
      // composerCardClass glass: rounded-md + blurred translucent fill.
      expect(cls).toContain("rounded-md");
      expect(cls).toContain("backdrop-blur-[12px]");
      expect(cls).not.toContain("rounded-full");
      // Empty capsules keep the same glass — no opacity change.
      expect(cls).not.toContain("opacity-60");
    }
  });

  it("surfaces the live counts in the accessible name and the hover label", () => {
    useBashStore
      .getState()
      .applySnapshot("s1", { jobs: [bashJob, bashJob2], waits: [] });
    useSessionStore.setState({ sessions: [subagentSession()] });
    seedTurn("s1", {
      todoItems: [{ id: "t1", content: "do a", status: "pending" }],
      todoPending: 1,
    });

    render(<SessionStatusLine sessionId="s1" />);
    expect(
      screen.getByRole("button", { name: "Terminal status, 2 active" }),
    ).toBeTruthy();
    expect(
      screen.getByRole("button", { name: "Subagent status, 1 running" }),
    ).toBeTruthy();
    expect(
      screen.getByRole("button", { name: "Task status, 1 task" }),
    ).toBeTruthy();

    // Todo owns the horizontal slot by default, so its count is already
    // visible; hovering another capsule collapses it (exactly one expanded).
    const todo = screen.getByTestId("capsule-todo");
    expect(within(todo).getByText("Tasks")).toBeTruthy();
    expect(within(todo).getByText("0/1")).toBeTruthy();
    fireEvent.mouseEnter(screen.getByTestId("capsule-terminal"));
    expect(
      screen
        .getByTestId("capsule-todo")
        .querySelector('[data-content-hidden="true"]'),
    ).toBeTruthy();
    expect(
      within(screen.getByTestId("capsule-terminal")).getByText("×2"),
    ).toBeTruthy();
  });

  it("counts subagent children from the session list after a reload (no bindings)", () => {
    // bindings live only in memory (bound events never replay); the child rows
    // in `session/list` are durable, so the capsule total must come from there.
    useSessionStore.setState({
      sessions: [
        {
          id: "ch1",
          project: "E:\\p",
          updated_at: 0,
          preview: "p",
          running: true,
          turn: null,
          agent_id: "researcher",
          api_model_id: "m",
          parent_session_id: "s1",
          parent_call_id: "c1",
        },
        {
          id: "ch2",
          project: "E:\\p",
          updated_at: 0,
          preview: "p",
          running: false,
          turn: null,
          agent_id: "explorer",
          api_model_id: "m",
          parent_session_id: "s1",
          parent_call_id: "c2",
        },
      ],
    });

    render(<SessionStatusLine sessionId="s1" />);
    expect(
      screen.getByRole("button", { name: "Subagent status, 1 running" }),
    ).toBeTruthy();
    expect(
      within(screen.getByTestId("capsule-subagent")).getByText("1/2 running"),
    ).toBeTruthy();
  });
});

describe("SessionStatusLine — level 1 horizontal expansion", () => {
  it("keeps exactly one capsule expanded, defaulting to the first", () => {
    render(<SessionStatusLine sessionId="s1" />);

    // Todo owns the slot by default; the other three are icon-only.
    expect(screen.getByTestId("capsule-todo").dataset.expanded).toBe("true");
    expect(
      within(screen.getByTestId("capsule-todo")).getByText("Tasks"),
    ).toBeTruthy();
    for (const id of ["plan", "subagent", "terminal"] as const) {
      expect(screen.getByTestId(`capsule-${id}`).dataset.expanded).toBe("false");
    }
    const plan = screen.getByTestId("capsule-plan");
    expect(
      plan.querySelector('[data-content-hidden="true"]'),
    ).toBeTruthy();
  });

  it("hover claims the slot and mouse-leave keeps it (sticky)", () => {
    render(<SessionStatusLine sessionId="s1" />);
    const plan = screen.getByTestId("capsule-plan");

    fireEvent.mouseEnter(plan);
    expect(plan.dataset.expanded).toBe("true");
    expect(within(plan).getByText("Plan")).toBeTruthy();
    // Exactly one: the previous owner collapses immediately.
    expect(screen.getByTestId("capsule-todo").dataset.expanded).toBe("false");

    fireEvent.mouseLeave(plan);
    // Sticky — the slot does not snap back.
    expect(plan.dataset.expanded).toBe("true");
    expect(within(plan).getByText("Plan")).toBeTruthy();
  });

  it("an idle data change claims the slot for that capsule's domain", () => {
    vi.useFakeTimers();
    seedRows("s1", [
      bashCallRow("c1", { command: "sleep 1", run_in_background: true }),
    ]);
    render(<SessionStatusLine sessionId="s1" />);
    // Move the slot off todo first; hover claims are sticky.
    const plan = screen.getByTestId("capsule-plan");
    fireEvent.mouseEnter(plan);
    fireEvent.mouseLeave(plan);
    expect(plan.dataset.expanded).toBe("true");

    // Idle (no hover, no panel): a background terminal claims the slot, but
    // only after the short-call grace window.
    act(() => {
      useBashStore
        .getState()
        .applySnapshot("s1", { jobs: [bashJob], waits: [] });
    });
    expect(screen.getByTestId("capsule-terminal").dataset.expanded).toBe(
      "false",
    );
    act(() => {
      vi.advanceTimersByTime(BASH_CLAIM_GRACE_MS + 1);
    });
    expect(screen.getByTestId("capsule-terminal").dataset.expanded).toBe("true");
    expect(screen.getByTestId("capsule-plan").dataset.expanded).toBe("false");
  });

  it("keeps a foreground bash call out of the capsule and the slot", () => {
    vi.useFakeTimers();
    // No `run_in_background`: a foreground call stays in the message list.
    seedRows("s1", [bashCallRow("c1", { command: "ls" })]);
    render(<SessionStatusLine sessionId="s1" />);

    act(() => {
      useBashStore
        .getState()
        .applySnapshot("s1", { jobs: [bashJob], waits: [] });
    });
    act(() => {
      vi.advanceTimersByTime(BASH_CLAIM_GRACE_MS * 3);
    });

    expect(screen.getByTestId("capsule-todo").dataset.expanded).toBe("true");
    const terminal = screen.getByTestId("capsule-terminal");
    expect(within(terminal).getByText("×0")).toBeTruthy();
    expect(within(terminal).getByText("No active terminals")).toBeTruthy();
  });

  it("never claims for a background job that dies inside the grace window", () => {
    vi.useFakeTimers();
    seedRows("s1", [
      bashCallRow("c1", { command: "npm run dev", run_in_background: true }),
    ]);
    render(<SessionStatusLine sessionId="s1" />);

    act(() => {
      useBashStore
        .getState()
        .applySnapshot("s1", { jobs: [bashJob], waits: [] });
    });
    act(() => {
      vi.advanceTimersByTime(BASH_CLAIM_GRACE_MS - 100);
    });
    act(() => {
      useBashStore.getState().applySnapshot("s1", { jobs: [], waits: [] });
    });
    act(() => {
      vi.advanceTimersByTime(BASH_CLAIM_GRACE_MS * 2);
    });
    expect(screen.getByTestId("capsule-todo").dataset.expanded).toBe("true");
  });

  it("does not claim the slot when a background job leaves", () => {
    vi.useFakeTimers();
    seedRows("s1", [
      bashCallRow("c1", { command: "npm run dev", run_in_background: true }),
    ]);
    useBashStore.getState().applySnapshot("s1", { jobs: [bashJob], waits: [] });
    render(<SessionStatusLine sessionId="s1" />);
    // The job was already present at mount: a baseline, never a claim.
    act(() => {
      vi.advanceTimersByTime(BASH_CLAIM_GRACE_MS * 2);
    });
    expect(screen.getByTestId("capsule-todo").dataset.expanded).toBe("true");

    act(() => {
      useBashStore.getState().applySnapshot("s1", { jobs: [], waits: [] });
    });
    act(() => {
      vi.advanceTimersByTime(BASH_CLAIM_GRACE_MS * 2);
    });
    expect(screen.getByTestId("capsule-todo").dataset.expanded).toBe("true");
  });

  it("claims the slot for a plan change too", () => {
    render(<SessionStatusLine sessionId="s1" />);
    expect(screen.getByTestId("capsule-todo").dataset.expanded).toBe("true");

    act(() => {
      seedTurn("s1", { activePlanPath: ".litecode/plan/calm.md" });
    });
    expect(screen.getByTestId("capsule-plan").dataset.expanded).toBe("true");
    expect(screen.getByTestId("capsule-todo").dataset.expanded).toBe("false");
  });

  it("consumes data changes seen while hovered instead of queueing them", () => {
    render(<SessionStatusLine sessionId="s1" />);
    const plan = screen.getByTestId("capsule-plan");
    fireEvent.mouseEnter(plan);

    // Lands while hovering: the slot must not be stolen.
    act(() => {
      seedTurn("s1", {
        todoItems: [{ id: "t1", content: "do a", status: "pending" }],
        todoPending: 1,
      });
    });
    expect(plan.dataset.expanded).toBe("true");

    // Leaving the hover does not replay the consumed change either.
    fireEvent.mouseLeave(plan);
    expect(plan.dataset.expanded).toBe("true");
    expect(screen.getByTestId("capsule-todo").dataset.expanded).toBe("false");
  });

  it("renders rich detail in the full-row expanded capsule", () => {
    useBashStore
      .getState()
      .applySnapshot("s1", { jobs: [bashJob, bashJob2], waits: [] });
    useSessionStore.setState({ sessions: [subagentSession()] });
    seedTurn("s1", {
      todoItems: [
        { id: "t1", content: "first", status: "in_progress" },
        { id: "t2", content: "second", status: "pending" },
      ],
      todoPending: 1,
      todoInProgress: 1,
    });

    render(<SessionStatusLine sessionId="s1" />);

    // Todo owns the slot by default: current task + progress count, and the
    // expanded capsule stretches to fill the row (flex-1).
    const todo = screen.getByTestId("capsule-todo");
    // The current task renders as plain text, so match textContent.
    expect(todo.textContent).toContain("first");
    expect(within(todo).getByText("0/2")).toBeTruthy();
    // Flexbox computes the responsive endpoint; the expanded capsule takes all
    // remaining space and animates the focus hand-off via flex-grow.
    expect(todo.style.flexBasis).toBe("36px");
    expect(todo.style.flexGrow).toBe("1");

    // Hover plan: full-row with its own detail (empty state here).
    fireEvent.mouseEnter(screen.getByTestId("capsule-plan"));
    const plan = screen.getByTestId("capsule-plan");
    expect(plan.style.flexGrow).toBe("1");
    expect(todo.style.flexGrow).toBe("0");
    expect(within(plan).getByText("No active plan")).toBeTruthy();
    expect(
      screen
        .getByTestId("capsule-todo")
        .querySelector('[data-content-hidden="true"]'),
    ).toBeTruthy();

    // Hover subagent: worker summary (running / total, no badge).
    fireEvent.mouseEnter(screen.getByTestId("capsule-subagent"));
    const sub = screen.getByTestId("capsule-subagent");
    expect(within(sub).getByText("1/1 running")).toBeTruthy();

    // Hover terminal: latest command preview + count.
    fireEvent.mouseEnter(screen.getByTestId("capsule-terminal"));
    const term = screen.getByTestId("capsule-terminal");
    expect(within(term).getByText("sleep 2")).toBeTruthy();
    expect(within(term).getByText("×2")).toBeTruthy();

    // Collapsed capsules keep the icon-only floor (shrink-0, no flex-1).
    expect(screen.getByTestId("capsule-todo").className).toContain("shrink-0");
    // Capsule is a <button>: UA styles center text, so the detail must be
    // explicitly left-aligned.
    expect(screen.getByTestId("capsule-todo").className).toContain("text-left");
  });

  it("expands only the hovered capsule (per-capsule state)", () => {
    render(<SessionStatusLine sessionId="s1" />);
    fireEvent.mouseEnter(screen.getByTestId("capsule-plan"));
    expect(
      within(screen.getByTestId("capsule-plan")).getByText("Plan"),
    ).toBeTruthy();
    expect(
      screen
        .getByTestId("capsule-todo")
        .querySelector('[data-content-hidden="true"]'),
    ).toBeTruthy();
  });

  it("counts bound subagents into the worker summary", () => {
    // One running child plus two more durable bindings → 1 running of 3 total.
    useSessionStore.setState({ sessions: [subagentSession()] });

    render(<SessionStatusLine sessionId="s1" />);
    // Landing the bindings after mount claims the worker slot while idle.
    act(() => {
      const onSubagentBound = useMessageStore.getState().onSubagentBound;
      for (const [callId, childId] of [
        ["c1", "ch1"],
        ["c2", "ch2"],
        ["c3", "ch3"],
      ] as const) {
        onSubagentBound("s1", {
          session_id: "s1",
          call_id: callId,
          child_session_id: childId,
        });
      }
    });
    expect(screen.getByTestId("capsule-subagent").dataset.expanded).toBe(
      "true",
    );
    expect(
      within(screen.getByTestId("capsule-subagent")).getByText("1/3 running"),
    ).toBeTruthy();
  });

  it("pins the capsule expanded while its vertical panel is open", () => {
    render(<SessionStatusLine sessionId="s1" />);
    const capsule = screen.getByTestId("capsule-plan");
    fireEvent.click(capsule);
    expect(screen.getByTestId("status-capsule-panel")).toBeTruthy();
    // Panel open keeps it labelled without hover.
    expect(capsule.dataset.expanded).toBe("true");
    expect(within(capsule).getByText("Plan")).toBeTruthy();
  });

  it("hover alone never opens a panel; an open panel follows hover", () => {
    render(<SessionStatusLine sessionId="s1" />);
    // Level-1 hover without a panel: horizontal slot only, no panel.
    fireEvent.mouseEnter(screen.getByTestId("capsule-plan"));
    expect(screen.queryByTestId("status-capsule-panel")).toBeNull();
    fireEvent.mouseLeave(screen.getByTestId("capsule-plan"));

    // Click opens the panel…
    fireEvent.click(screen.getByTestId("capsule-terminal"));
    expect(screen.getByTestId("status-capsule-panel").dataset.capsule).toBe(
      "terminal",
    );
    // …then hovering another capsule follows it onto the panel.
    fireEvent.mouseEnter(screen.getByTestId("capsule-plan"));
    expect(screen.getByTestId("status-capsule-panel").dataset.capsule).toBe(
      "plan",
    );
    // Pointer leaving the capsule keeps the panel on the last hovered one.
    fireEvent.mouseLeave(screen.getByTestId("capsule-plan"));
    expect(screen.getByTestId("status-capsule-panel").dataset.capsule).toBe(
      "plan",
    );
  });
});

describe("SessionStatusLine — vertical expand", () => {
  it("contains panel scrolling instead of chaining into the chat transcript", () => {
    render(<SessionStatusLine sessionId="s1" />);
    fireEvent.click(screen.getByTestId("capsule-plan"));

    expect(screen.getByTestId("status-panel-scroll").className).toContain(
      "overscroll-contain",
    );
    expect(screen.getByTestId("status-capsule-panel").className).toContain(
      "[container-type:size]",
    );
  });

  it("opens the clicked capsule's panel at the fixed initial height", () => {
    useBashStore.getState().applySnapshot("s1", { jobs: [bashJob], waits: [] });
    render(<SessionStatusLine sessionId="s1" />);

    expect(screen.queryByTestId("status-capsule-panel")).toBeNull();
    fireEvent.click(screen.getByTestId("capsule-terminal"));

    const panel = screen.getByTestId("status-capsule-panel");
    expect(panel.dataset.capsule).toBe("terminal");
    // The terminal panel opens taller: it hosts a live console, not a list.
    expect(panel.style.height).toBe(`${panelInitialHeight("terminal")}px`);
    // The alive job's compact header still carries the reveal affordance.
    expect(
      screen.getByRole("button", { name: "Reveal terminal: sleep 1" }),
    ).toBeTruthy();
  });

  it("renders the shared bash view with the live tail for a background job", async () => {
    const sendRpc = vi.fn(async (method: string) => {
      if (method === "bash/tail") {
        return {
          text: "live-out",
          truncated_on_disk: false,
          alive: true,
          exit_code: null,
        };
      }
      throw new Error(`unexpected rpc ${method}`);
    });
    useConnectionStore.setState({ state: "connected", sendRpc } as never);
    // The transcript supplies the full command + the sealed running doc, so
    // the view can address the job by its bash id.
    seedRows("s1", [
      bashCallRow("c1", {
        command: "npm run dev -- --host",
        run_in_background: true,
      }),
      bashResultRow(
        "c1",
        "status: running\nbash_id: bg_a\noutput_file: .litecode/bash/bg_a.output\n",
      ),
    ]);
    useBashStore.getState().applySnapshot("s1", { jobs: [bashJob], waits: [] });
    render(<SessionStatusLine sessionId="s1" />);

    fireEvent.click(screen.getByTestId("capsule-terminal"));

    const console = screen.getByTestId("bash-console");
    // The full command comes from the call arguments, not the 80-char preview.
    expect(within(console).getByText("npm run dev -- --host")).toBeTruthy();
    expect(await screen.findByText("live-out")).toBeTruthy();
    await waitFor(() => {
      expect(sendRpc).toHaveBeenCalledWith("bash/tail", { bash_id: "bg_a" });
    });
  });

  it("lists only background jobs in the terminal panel", () => {
    // A foreground call of the same session must not lease a panel slot.
    seedRows("s1", [bashCallRow("c1", { command: "ls" })]);
    useBashStore.getState().applySnapshot("s1", { jobs: [bashJob], waits: [] });
    render(<SessionStatusLine sessionId="s1" />);

    fireEvent.click(screen.getByTestId("capsule-terminal"));

    const panel = screen.getByTestId("status-capsule-panel");
    expect(within(panel).getByText("No active terminals")).toBeTruthy();
    expect(screen.queryByTestId("terminal-job-bg_a")).toBeNull();
  });

  it("kills a live terminal from the panel", () => {
    const sendRpc = vi.fn(async (method: string) =>
      method === "bash/tail"
        ? { text: "", truncated_on_disk: false, alive: true, exit_code: null }
        : { ok: true },
    );
    useConnectionStore.setState({ state: "connected", sendRpc } as never);
    useBashStore.getState().applySnapshot("s1", { jobs: [bashJob], waits: [] });
    render(<SessionStatusLine sessionId="s1" />);

    fireEvent.click(screen.getByTestId("capsule-terminal"));
    fireEvent.click(screen.getByRole("button", { name: /^Kill$/ }));

    expect(sendRpc).toHaveBeenCalledWith("bash/kill", { bash_id: "bg_a" });
  });

  it("closes the panel when its capsule is clicked again", () => {
    vi.useFakeTimers();
    render(<SessionStatusLine sessionId="s1" />);
    fireEvent.click(screen.getByTestId("capsule-plan"));
    expect(screen.getByTestId("status-capsule-panel")).toBeTruthy();
    fireEvent.click(screen.getByTestId("capsule-plan"));
    // The close keeps the mount for its shrink-back animation…
    const panel = screen.getByTestId("status-capsule-panel");
    expect(panel.dataset.capsule).toBe("plan");
    // …which tears it down when the exit animation completes.
    act(() => {
      vi.advanceTimersByTime(PANEL_EXIT_MS + 100);
    });
    expect(screen.queryByTestId("status-capsule-panel")).toBeNull();
  });

  it("allows only one capsule panel open at a time (mutual exclusion)", () => {
    render(<SessionStatusLine sessionId="s1" />);
    fireEvent.click(screen.getByTestId("capsule-terminal"));
    expect(screen.getByTestId("status-capsule-panel").dataset.capsule).toBe(
      "terminal",
    );

    fireEvent.click(screen.getByTestId("capsule-todo"));
    const panels = screen.getAllByTestId("status-capsule-panel");
    expect(panels).toHaveLength(1);
    expect(panels[0].dataset.capsule).toBe("todo");
  });

  it("keeps only the newly-clicked capsule open in the reverse order (B→A)", () => {
    render(<SessionStatusLine sessionId="s1" />);
    fireEvent.click(screen.getByTestId("capsule-subagent"));
    expect(screen.getByTestId("status-capsule-panel").dataset.capsule).toBe(
      "subagent",
    );

    fireEvent.click(screen.getByTestId("capsule-terminal"));
    const panels = screen.getAllByTestId("status-capsule-panel");
    expect(panels).toHaveLength(1);
    expect(panels[0].dataset.capsule).toBe("terminal");
  });

  it("closes the panel on an outside mousedown", () => {
    vi.useFakeTimers();
    render(<SessionStatusLine sessionId="s1" />);
    fireEvent.click(screen.getByTestId("capsule-todo"));
    expect(screen.getByTestId("status-capsule-panel")).toBeTruthy();
    fireEvent.mouseDown(document.body);
    // Close keeps the mount for the shrink-back animation…
    const panel = screen.getByTestId("status-capsule-panel");
    expect(panel.dataset.capsule).toBe("todo");
    // …and the exit completion tears it down.
    act(() => {
      vi.advanceTimersByTime(PANEL_EXIT_MS + 100);
    });
    expect(screen.queryByTestId("status-capsule-panel")).toBeNull();
  });

  it("closes the panel on Escape", () => {
    vi.useFakeTimers();
    render(<SessionStatusLine sessionId="s1" />);
    fireEvent.click(screen.getByTestId("capsule-todo"));
    expect(screen.getByTestId("status-capsule-panel")).toBeTruthy();
    fireEvent.keyDown(document, { key: "Escape" });
    const panel = screen.getByTestId("status-capsule-panel");
    expect(panel.dataset.capsule).toBe("todo");
    act(() => {
      vi.advanceTimersByTime(PANEL_EXIT_MS + 100);
    });
    expect(screen.queryByTestId("status-capsule-panel")).toBeNull();
  });

  it("animates the panel in on open and out on close", () => {
    vi.useFakeTimers();
    render(<SessionStatusLine sessionId="s1" />);
    fireEvent.click(screen.getByTestId("capsule-terminal"));
    const opened = screen.getByTestId("status-capsule-panel");
    expect(opened.className).toContain("status-panel-enter");
    expect(opened.className).not.toContain("status-panel-exit");
    // Origin tracks the owning capsule's slot (jsdom rects are zero, so it
    // falls back to the row's left edge + half a collapsed pill).
    expect(opened.style.transformOrigin).toBe("18px 100%");

    fireEvent.click(screen.getByTestId("capsule-terminal"));
    const closing = screen.getByTestId("status-capsule-panel");
    expect(closing.className).toContain("status-panel-exit");
    act(() => {
      vi.advanceTimersByTime(PANEL_EXIT_MS + 100);
    });
    expect(screen.queryByTestId("status-capsule-panel")).toBeNull();
  });

  it("switching capsules mounts the new panel directly (no exit overlap)", () => {
    render(<SessionStatusLine sessionId="s1" />);
    fireEvent.click(screen.getByTestId("capsule-terminal"));
    fireEvent.click(screen.getByTestId("capsule-plan"));
    const panel = screen.getByTestId("status-capsule-panel");
    expect(panel.dataset.capsule).toBe("plan");
    // The previous mount is torn down instantly — one panel, entering only.
    expect(screen.getAllByTestId("status-capsule-panel")).toHaveLength(1);
    expect(panel.className).toContain("status-panel-enter");
    expect(panel.className).not.toContain("status-panel-exit");
  });

  it("reopens during the exit animation instead of queueing a close", () => {
    vi.useFakeTimers();
    render(<SessionStatusLine sessionId="s1" />);
    fireEvent.click(screen.getByTestId("capsule-todo"));
    fireEvent.click(screen.getByTestId("capsule-todo")); // closing…
    const closing = screen.getByTestId("status-capsule-panel");
    expect(closing.className).toContain("status-panel-exit");
    fireEvent.click(screen.getByTestId("capsule-terminal")); // reopen another
    const reopened = screen.getByTestId("status-capsule-panel");
    expect(reopened.dataset.capsule).toBe("terminal");
    expect(reopened.className).toContain("status-panel-enter");
    // The reopen cancels the stale closing timer — the new panel stays.
    act(() => {
      vi.advanceTimersByTime(PANEL_EXIT_MS + 100);
    });
    expect(screen.getByTestId("status-capsule-panel").dataset.capsule).toBe(
      "terminal",
    );
  });
});

describe("SessionStatusLine — drag handle", () => {
  it("grows the panel taller when the handle is dragged up", () => {
    render(<SessionStatusLine sessionId="s1" />);
    fireEvent.click(screen.getByTestId("capsule-todo"));

    const panel = screen.getByTestId("status-capsule-panel");
    const handle = screen.getByTestId("status-panel-resize");
    expect(panel.style.height).toBe(`${PANEL_INITIAL_H}px`);

    fireEvent.pointerDown(handle, { pointerId: 1, clientY: 400 });
    fireEvent.pointerMove(handle, { pointerId: 1, clientY: 300 });
    expect(panel.style.height).toBe(`${PANEL_INITIAL_H + 100}px`);

    // Dragging down shrinks it back; the panel never goes below the floor.
    fireEvent.pointerMove(handle, { pointerId: 1, clientY: 900 });
    expect(panel.style.height).toBe("80px");

    fireEvent.pointerUp(handle, { pointerId: 1, clientY: 900 });
  });

  it("ignores pointer moves when no drag is in progress", () => {
    render(<SessionStatusLine sessionId="s1" />);
    fireEvent.click(screen.getByTestId("capsule-todo"));
    const panel = screen.getByTestId("status-capsule-panel");
    fireEvent.pointerMove(screen.getByTestId("status-panel-resize"), {
      pointerId: 1,
      clientY: 100,
    });
    expect(panel.style.height).toBe(`${PANEL_INITIAL_H}px`);
  });

  it("clamps the panel height at the maximum when dragged far up", () => {
    render(<SessionStatusLine sessionId="s1" />);
    fireEvent.click(screen.getByTestId("capsule-todo"));
    const panel = screen.getByTestId("status-capsule-panel");
    const handle = screen.getByTestId("status-panel-resize");

    fireEvent.pointerDown(handle, { pointerId: 1, clientY: 1000 });
    fireEvent.pointerMove(handle, { pointerId: 1, clientY: 0 });
    expect(panel.style.height).toBe(`${PANEL_MAX_H}px`);

    fireEvent.pointerUp(handle, { pointerId: 1, clientY: 0 });
  });

  it("resets to the fixed initial height when switching capsules", () => {
    render(<SessionStatusLine sessionId="s1" />);
    fireEvent.click(screen.getByTestId("capsule-todo"));
    const handle = screen.getByTestId("status-panel-resize");
    fireEvent.pointerDown(handle, { pointerId: 1, clientY: 400 });
    fireEvent.pointerMove(handle, { pointerId: 1, clientY: 300 });
    fireEvent.pointerUp(handle, { pointerId: 1, clientY: 300 });
    expect(screen.getByTestId("status-capsule-panel").style.height).toBe(
      `${PANEL_INITIAL_H + 100}px`,
    );

    fireEvent.click(screen.getByTestId("capsule-terminal"));
    const panel = screen.getByTestId("status-capsule-panel");
    expect(panel.dataset.capsule).toBe("terminal");
    expect(panel.style.height).toBe(`${panelInitialHeight("terminal")}px`);
  });

  it("releases the body drag lock on pointerup", () => {
    render(<SessionStatusLine sessionId="s1" />);
    fireEvent.click(screen.getByTestId("capsule-todo"));
    const handle = screen.getByTestId("status-panel-resize");

    fireEvent.pointerDown(handle, { pointerId: 1, clientY: 400 });
    expect(document.body.style.cursor).toBe("ns-resize");
    expect(document.body.style.userSelect).toBe("none");

    fireEvent.pointerUp(handle, { pointerId: 1, clientY: 400 });
    expect(document.body.style.cursor).toBe("");
    expect(document.body.style.userSelect).toBe("");
  });

  it("releases the body drag lock when the panel closes mid-drag (Esc)", () => {
    vi.useFakeTimers();
    render(<SessionStatusLine sessionId="s1" />);
    fireEvent.click(screen.getByTestId("capsule-todo"));
    const handle = screen.getByTestId("status-panel-resize");

    fireEvent.pointerDown(handle, { pointerId: 1, clientY: 400 });
    expect(document.body.style.cursor).toBe("ns-resize");

    // Esc closes the panel; the drag lock is released immediately even though
    // the closing mount (with its handle) lingers for the exit animation.
    fireEvent.keyDown(document, { key: "Escape" });
    expect(screen.getByTestId("status-capsule-panel")).toBeTruthy();
    expect(document.body.style.cursor).toBe("");
    expect(document.body.style.userSelect).toBe("");
    act(() => {
      vi.advanceTimersByTime(PANEL_EXIT_MS + 100);
    });
    expect(screen.queryByTestId("status-capsule-panel")).toBeNull();
  });

  it("releases the body drag lock when the component unmounts mid-drag", () => {
    const { unmount } = render(<SessionStatusLine sessionId="s1" />);
    fireEvent.click(screen.getByTestId("capsule-todo"));
    const handle = screen.getByTestId("status-panel-resize");

    fireEvent.pointerDown(handle, { pointerId: 1, clientY: 400 });
    expect(document.body.style.cursor).toBe("ns-resize");

    unmount();
    expect(document.body.style.cursor).toBe("");
    expect(document.body.style.userSelect).toBe("");
  });
});

describe("SessionStatusLine — migrated chip content", () => {
  it("reveals a bash job from the terminal panel", () => {
    const onRevealBash = vi.fn();
    useBashStore
      .getState()
      .applySnapshot("s1", { jobs: [bashJob, bashJob2], waits: [] });
    render(<SessionStatusLine sessionId="s1" onRevealBash={onRevealBash} />);

    fireEvent.click(screen.getByTestId("capsule-terminal"));
    fireEvent.click(
      screen.getByRole("button", { name: "Reveal terminal: sleep 2" }),
    );
    expect(onRevealBash).toHaveBeenCalledWith("c2");
  });

  it("lists every bound subagent session in the subagent panel", () => {
    useMessageStore.getState().onSubagentBound("s1", {
      session_id: "s1",
      call_id: "call_a",
      child_session_id: "child-a",
    });
    render(<SessionStatusLine sessionId="s1" />);

    fireEvent.click(screen.getByTestId("capsule-subagent"));
    const panel = screen.getByTestId("status-capsule-panel");
    expect(within(panel).getByTestId("subagent-roster")).toBeTruthy();
    expect(
      within(panel).getByRole("button", { name: "Subagent subagent" }),
    ).toBeTruthy();
  });

  it("shows an empty state in each panel when its family has no data", () => {
    render(<SessionStatusLine sessionId="s1" />);
    const panel = () => screen.getByTestId("status-capsule-panel");
    fireEvent.click(screen.getByTestId("capsule-terminal"));
    expect(within(panel()).getByText("No active terminals")).toBeTruthy();
    fireEvent.click(screen.getByTestId("capsule-subagent"));
    expect(within(panel()).getByText("No subagents")).toBeTruthy();
    fireEvent.click(screen.getByTestId("capsule-todo"));
    expect(within(panel()).getByText("No tasks yet")).toBeTruthy();
    fireEvent.click(screen.getByTestId("capsule-plan"));
    expect(within(panel()).getByText("No active plan")).toBeTruthy();
  });

  it("opens the workspace-relative active plan from the plan panel", () => {
    const openFile = vi.fn(async () => {});
    useEditorStore.setState({ openFile } as never);
    useSessionStore.setState({ project: "E:\\project" } as never);
    seedTurn("s1", { activePlanPath: ".litecode/plan/calm.md" });

    render(<SessionStatusLine sessionId="s1" />);
    fireEvent.click(screen.getByTestId("capsule-plan"));
    fireEvent.click(screen.getByRole("button", { name: "Open plan" }));
    expect(openFile).toHaveBeenCalledWith(".litecode/plan/calm.md");
  });

  it("执行计划 fires a plan_execution turn for the active plan", () => {
    const sendRpc = vi.fn(async () => ({ started: true }));
    useConnectionStore.setState({ state: "connected", sendRpc } as never);
    seedTurn("s1", { activePlanPath: ".litecode/plan/calm.md" });

    render(<SessionStatusLine sessionId="s1" />);
    fireEvent.click(screen.getByTestId("capsule-plan"));
    fireEvent.click(screen.getByRole("button", { name: "执行计划" }));

    expect(sendRpc).toHaveBeenCalledWith("agent/run", {
      input: PLAN_EXECUTE_PROMPT,
      session_id: "s1",
      plan_execution: true,
    });
  });

  it("disables 执行计划 while a turn is already running", () => {
    seedTurn("s1", {
      activePlanPath: ".litecode/plan/calm.md",
      runState: "running",
    });

    render(<SessionStatusLine sessionId="s1" />);
    fireEvent.click(screen.getByTestId("capsule-plan"));
    const button = screen.getByRole("button", {
      name: "执行计划",
    }) as HTMLButtonElement;
    expect(button.disabled).toBe(true);
  });

  it("renders the plan file's markdown in the plan panel", async () => {
    useSessionStore.setState({ project: "E:\\project" } as never);
    vi.mocked(readFile).mockResolvedValue("# Title\n\nbody text");
    seedTurn("s1", { activePlanPath: ".litecode/plan/calm.md" });

    render(<SessionStatusLine sessionId="s1" />);
    fireEvent.click(screen.getByTestId("capsule-plan"));

    expect(vi.mocked(readFile)).toHaveBeenCalledWith(".litecode/plan/calm.md");
    expect(await screen.findByRole("heading", { name: "Title" })).toBeTruthy();
    expect(screen.getByText("body text")).toBeTruthy();
  });

  it("shows lost when the plan file cannot be read", async () => {
    vi.mocked(readFile).mockRejectedValue(new Error("ENOENT"));
    seedTurn("s1", { activePlanPath: ".litecode/plan/gone.md" });

    render(<SessionStatusLine sessionId="s1" />);
    fireEvent.click(screen.getByTestId("capsule-plan"));

    expect(await screen.findByText("lost")).toBeTruthy();
  });

  it("re-reads the plan when the watcher reports the file changed", async () => {
    useSessionStore.setState({ project: "E:\\project" } as never);
    vi.mocked(readFile).mockResolvedValueOnce("# v1");
    seedTurn("s1", { activePlanPath: ".litecode/plan/calm.md" });

    render(<SessionStatusLine sessionId="s1" />);
    fireEvent.click(screen.getByTestId("capsule-plan"));
    expect(await screen.findByRole("heading", { name: "v1" })).toBeTruthy();

    vi.mocked(readFile).mockResolvedValueOnce("# v2");
    act(() => {
      useWorkspaceChangeStore
        .getState()
        .record([".litecode/plan/calm.md"], "modified");
    });

    expect(await screen.findByRole("heading", { name: "v2" })).toBeTruthy();
    expect(vi.mocked(readFile)).toHaveBeenCalledTimes(2);
  });

  it("marks the plan lost when the watcher reports it deleted", async () => {
    useSessionStore.setState({ project: "E:\\project" } as never);
    vi.mocked(readFile).mockResolvedValue("# v1");
    seedTurn("s1", { activePlanPath: ".litecode/plan/gone.md" });

    render(<SessionStatusLine sessionId="s1" />);
    fireEvent.click(screen.getByTestId("capsule-plan"));
    expect(await screen.findByRole("heading", { name: "v1" })).toBeTruthy();

    act(() => {
      useWorkspaceChangeStore
        .getState()
        .record([".litecode/plan/gone.md"], "deleted");
    });

    expect(await screen.findByText("lost")).toBeTruthy();
  });

  it("does not re-read on mount when the latest workspace tick is already known", async () => {
    useSessionStore.setState({ project: "E:\\project" } as never);
    useWorkspaceChangeStore
      .getState()
      .record([".litecode/plan/calm.md"], "modified");
    vi.mocked(readFile).mockResolvedValue("# current");
    seedTurn("s1", { activePlanPath: ".litecode/plan/calm.md" });

    render(<SessionStatusLine sessionId="s1" />);
    fireEvent.click(screen.getByTestId("capsule-plan"));

    expect(await screen.findByRole("heading", { name: "current" })).toBeTruthy();
    expect(vi.mocked(readFile)).toHaveBeenCalledTimes(1);
  });

  it("ignores a stale in-flight read after the watcher reports deletion", async () => {
    useSessionStore.setState({ project: "E:\\project" } as never);
    let resolveRead: (value: string) => void = () => {};
    vi.mocked(readFile).mockImplementationOnce(
      () =>
        new Promise<string>((resolve) => {
          resolveRead = resolve;
        }),
    );
    seedTurn("s1", { activePlanPath: ".litecode/plan/gone.md" });

    render(<SessionStatusLine sessionId="s1" />);
    fireEvent.click(screen.getByTestId("capsule-plan"));

    act(() => {
      useWorkspaceChangeStore
        .getState()
        .record([".litecode/plan/gone.md"], "deleted");
    });
    expect(await screen.findByText("lost")).toBeTruthy();

    await act(async () => {
      resolveRead("# stale");
      await Promise.resolve();
    });
    expect(screen.queryByRole("heading", { name: "stale" })).toBeNull();
    expect(screen.getByText("lost")).toBeTruthy();
  });

  it("sits the Open affordance at the end of the plan file row", async () => {
    seedTurn("s1", { activePlanPath: ".litecode/plan/calm.md" });
    render(<SessionStatusLine sessionId="s1" />);
    fireEvent.click(screen.getByTestId("capsule-plan"));

    const row = await screen.findByTestId("plan-file-row");
    expect(within(row).getByText(".litecode/plan/calm.md")).toBeTruthy();
    const open = within(row).getByRole("button", { name: "Open plan" });
    expect(row.lastElementChild).toBe(open);
  });

  it("renders the todo list content in the todo panel", () => {
    seedTurn("s1", {
      todoItems: [
        { id: "t1", content: "first", status: "completed" },
        { id: "t2", content: "second", status: "in_progress" },
        { id: "t3", content: "third", status: "pending" },
      ],
      todoPending: 1,
      todoInProgress: 1,
      todoCompleted: 1,
    });
    render(<SessionStatusLine sessionId="s1" />);

    fireEvent.click(screen.getByTestId("capsule-todo"));
    const panel = screen.getByTestId("status-capsule-panel");
    expect(within(panel).getByText("first")).toBeTruthy();
    expect(within(panel).getByText("third")).toBeTruthy();
    expect(within(panel).getByText("1/3")).toBeTruthy();
  });

  it("shows the in_progress task only in the panel header, not the list", () => {
    seedTurn("s1", {
      todoItems: [
        { id: "t1", content: "do a", status: "in_progress" },
        { id: "t2", content: "do b", status: "pending" },
      ],
      todoPending: 1,
      todoInProgress: 1,
    });
    render(<SessionStatusLine sessionId="s1" />);
    fireEvent.click(screen.getByTestId("capsule-todo"));
    const panel = screen.getByTestId("status-capsule-panel");
    // Header: current task plain text + progress count.
    expect(panel.textContent).toContain("do a");
    expect(within(panel).getByText("0/2")).toBeTruthy();
    // List rows: only the non-in_progress items — "do a" appears exactly
    // once (the header), with no duplicated list row.
    expect(within(panel).getAllByText("do a")).toHaveLength(1);
    expect(within(panel).getByText("do b")).toBeTruthy();
  });
});

describe("SessionStatusLine — subagent roster panel (dock)", () => {
  const PARENT = "s1";
  const CHILD_A = "child-a";
  const CHILD_B = "child-b";

  function bind(callId: string, childId: string) {
    useMessageStore.getState().onSubagentBound(PARENT, {
      session_id: PARENT,
      call_id: callId,
      child_session_id: childId,
    });
  }

  function openRoster(): HTMLElement {
    render(<SessionStatusLine sessionId={PARENT} />);
    fireEvent.click(screen.getByTestId("capsule-subagent"));
    return screen.getByTestId("subagent-roster");
  }

  beforeEach(() => {
    useConnectionStore.setState({ state: "connected" });
  });

  afterEach(() => {
    setDockviewApi(null);
    useConnectionStore.setState({ state: "disconnected" });
    useMessageStore.getState().reset(PARENT);
    useMessageStore.getState().reset(CHILD_A);
    useMessageStore.getState().reset(CHILD_B);
    useTurnStore.getState().resetTurn(CHILD_A);
    useTurnStore.getState().resetTurn(CHILD_B);
    vi.restoreAllMocks();
  });

  it("lists every bound subagent session, deduped by child id", () => {
    bind("call_a", CHILD_A);
    bind("call_b", CHILD_B);
    bind("call_dup", CHILD_A);

    const roster = openRoster();
    expect(within(roster).getAllByRole("button")).toHaveLength(2);
  });

  it("labels a running subagent from its child session", () => {
    bind("call_a", CHILD_A);
    useSessionStore.setState({
      sessions: [subagentSession(CHILD_A, PARENT, true, "explore")],
    });

    const roster = openRoster();
    expect(
      within(roster).getByRole("button", { name: "Subagent explore" }),
    ).toBeTruthy();
    expect(within(roster).getByTestId("subagent-roster-running")).toBeTruthy();
  });

  it("labels a finished subagent from the child's last turn reason", () => {
    bind("call_b", CHILD_B);
    useSessionStore.setState({
      sessions: [
        {
          ...subagentSession(CHILD_B, PARENT, false, "worker"),
          last_turn_reason: "completed",
        },
      ],
    });

    const roster = openRoster();
    expect(
      within(roster).getByRole("button", { name: "Subagent worker" }),
    ).toBeTruthy();
    expect(
      within(roster).getByTestId("subagent-roster-finished").textContent,
    ).toBe("completed");
  });

  it("opens the child in its own read-only dock panel on row click (no embedded transcript)", () => {
    bind("call_a", CHILD_A);
    const addPanel = vi.fn();
    setDockviewApi({
      getPanel: () => undefined,
      addPanel,
      addGroup: vi.fn(() => ({ id: "g-new" })),
      groups: [{ api: { location: { type: "grid" }, id: "g1" } }],
    } as never);

    const roster = openRoster();
    // The roster no longer embeds the child transcript.
    expect(screen.queryByTestId("message-list")).toBeNull();

    fireEvent.click(
      within(roster).getByRole("button", { name: "Subagent subagent" }),
    );

    expect(addPanel).toHaveBeenCalledWith(
      expect.objectContaining({
        id: `subagent-${CHILD_A}`,
        component: "subagent",
        params: { sessionId: CHILD_A },
      }),
    );
    // Still no embedded transcript after the click — it lives in the dock panel.
    expect(screen.queryByTestId("message-list")).toBeNull();
  });
});

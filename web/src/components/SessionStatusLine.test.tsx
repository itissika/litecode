import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  within,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../api/workspace", () => ({ readFile: vi.fn() }));

import type { BashJob, SubagentJob } from "../api/types";
import { readFile } from "../api/workspace";
import { useBashStore } from "../stores/bashStore";
import { setDockviewApi, useConnectionStore } from "../stores/connectionStore";
import { useEditorStore } from "../stores/editorStore";
import { useMessageStore } from "../stores/messageStore";
import { useSessionStore } from "../stores/sessionStore";
import { useSubagentStore } from "../stores/subagentStore";
import { emptySlice, useTurnStore, type TurnSlice } from "../stores/turnStore";
import {
  PANEL_EXIT_MS,
  PANEL_INITIAL_H,
  PANEL_MAX_H,
  SessionStatusLine,
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

const subJob: SubagentJob = {
  id: "sa_a",
  call_id: "sc1",
  agent_name: "researcher",
  prompt_preview: "find the wiring",
  started_at_ms: Date.now(),
};

function seedTurn(sessionId: string, patch: Partial<TurnSlice>) {
  useTurnStore.setState({
    byId: new Map([[sessionId, { ...emptySlice(), ...patch }]]),
  });
}

const originalOpenFile = useEditorStore.getState().openFile;

beforeEach(() => {
  useBashStore.getState().reset();
  useSubagentStore.getState().reset();
  useMessageStore.setState({ bySession: new Map() });
  useTurnStore.setState({ byId: new Map() });
  useSessionStore.setState({ project: null, sessions: [], byId: new Map() } as never);
  useEditorStore.setState({ openFile: originalOpenFile } as never);
  vi.mocked(readFile).mockReset().mockResolvedValue("# plan");
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  useBashStore.getState().reset();
  useSubagentStore.getState().reset();
  useMessageStore.setState({ bySession: new Map() });
  useTurnStore.setState({ byId: new Map() });
  useSessionStore.setState({ project: null, sessions: [], byId: new Map() } as never);
  useEditorStore.setState({ openFile: originalOpenFile } as never);
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
    useSubagentStore.getState().applySnapshot("s1", { jobs: [subJob], waits: [] });
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
    render(<SessionStatusLine sessionId="s1" />);
    // Move the slot off todo first; hover claims are sticky.
    const plan = screen.getByTestId("capsule-plan");
    fireEvent.mouseEnter(plan);
    fireEvent.mouseLeave(plan);
    expect(plan.dataset.expanded).toBe("true");

    // Idle (no hover, no panel): a bash change claims terminal.
    act(() => {
      useBashStore
        .getState()
        .applySnapshot("s1", { jobs: [bashJob], waits: [] });
    });
    expect(screen.getByTestId("capsule-terminal").dataset.expanded).toBe("true");
    expect(screen.getByTestId("capsule-plan").dataset.expanded).toBe("false");
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
    useSubagentStore.getState().applySnapshot("s1", { jobs: [subJob], waits: [] });
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
    // WaveText fragments the current task into char spans, so match textContent.
    expect(todo.textContent).toContain("first");
    expect(within(todo).getByText("0/2")).toBeTruthy();
    // The expanded capsule's width is set inline (animated px, flex reflow
    // cannot transition) rather than a flex-1 class.
    expect(todo.style.width).toMatch(/^\d+px$/);

    // Hover plan: full-row with its own detail (empty state here).
    fireEvent.mouseEnter(screen.getByTestId("capsule-plan"));
    const plan = screen.getByTestId("capsule-plan");
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
    // A live job plus two more durable bindings → 1 running of 3 total.
    useSubagentStore
      .getState()
      .applySnapshot("s1", { jobs: [subJob], waits: [] });

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
  it("opens the clicked capsule's panel at the fixed initial height", () => {
    useBashStore.getState().applySnapshot("s1", { jobs: [bashJob], waits: [] });
    render(<SessionStatusLine sessionId="s1" />);

    expect(screen.queryByTestId("status-capsule-panel")).toBeNull();
    fireEvent.click(screen.getByTestId("capsule-terminal"));

    const panel = screen.getByTestId("status-capsule-panel");
    expect(panel.dataset.capsule).toBe("terminal");
    expect(panel.style.height).toBe(`${PANEL_INITIAL_H}px`);
    // Migrated terminal content: the alive job, clickable to reveal.
    expect(
      screen.getByRole("button", { name: "Reveal terminal: sleep 1" }),
    ).toBeTruthy();
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
    expect(panel.style.height).toBe(`${PANEL_INITIAL_H}px`);
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
    // Header: current task WaveText (char-fragmented, spaces as \u00a0) +
    // progress count.
    expect(panel.textContent).toContain("do\u00a0a");
    expect(within(panel).getByText("0/2")).toBeTruthy();
    // List rows: only the non-in_progress items — no plain "do a" row.
    expect(within(panel).queryByText("do a")).toBeNull();
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

  function seedParentRow(seq: number, kind: string, body: unknown) {
    useMessageStore.getState().onBufferItem(PARENT, {
      session_id: PARENT,
      seq,
      kind,
      body,
    } as never);
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

  it("labels a running subagent from its live job", () => {
    bind("call_a", CHILD_A);
    useSubagentStore.getState().applySnapshot(PARENT, {
      jobs: [
        {
          id: "j1",
          call_id: "call_a",
          agent_name: "explore",
          prompt_preview: "find it",
          started_at_ms: Date.now(),
        },
      ],
      waits: [],
    });

    const roster = openRoster();
    expect(
      within(roster).getByRole("button", { name: "Subagent explore" }),
    ).toBeTruthy();
    expect(within(roster).getByTestId("subagent-roster-running")).toBeTruthy();
  });

  it("labels a finished subagent from the parent transcript", () => {
    bind("call_b", CHILD_B);
    seedParentRow(0, "item/tool_call", {
      type: "function_call",
      id: "fc_b",
      call_id: "call_b",
      name: "subagent_launch",
      arguments: JSON.stringify({ agent: "worker", prompt: "do it" }),
      status: "completed",
    });
    seedParentRow(1, "item/tool_result", {
      type: "function_call_output",
      call_id: "call_b",
      output: "done",
    });

    const roster = openRoster();
    expect(
      within(roster).getByRole("button", { name: "Subagent worker" }),
    ).toBeTruthy();
    expect(
      within(roster).getByTestId("subagent-roster-finished").textContent,
    ).toBe("completed");
  });

  it("subscribes the child on expand and unsubscribes on collapse, keeping its slice", () => {
    bind("call_a", CHILD_A);
    // A loaded-but-empty window: `toSeq` marks the slice as hydrated, so a
    // collapse that dropped the projection would reset it back to 0.
    useMessageStore.getState().onBufferLoaded(CHILD_A, {
      session_id: CHILD_A,
      from_seq: 0,
      to_seq: 1,
      events: [],
    });
    const subscribe = vi
      .spyOn(useConnectionStore.getState(), "ensureSubscribe")
      .mockResolvedValue(undefined);
    const unsubscribe = vi.spyOn(
      useConnectionStore.getState(),
      "unsubscribeSession",
    );

    const roster = openRoster();
    const row = within(roster).getByRole("button", { name: "Subagent subagent" });
    fireEvent.click(row);

    expect(subscribe).toHaveBeenCalledWith(CHILD_A);
    // The child viewport mounted (empty session → its own empty state).
    expect(screen.getByText("Empty subagent session")).toBeTruthy();

    fireEvent.click(row);
    expect(unsubscribe).toHaveBeenCalledWith(CHILD_A);
    // P6: the projection stays resident for an incremental re-expand.
    expect(useMessageStore.getState().bySession.get(CHILD_A)?.toSeq).toBe(1);
  });

  it("does not unsubscribe a child that still has its own dock tab open", () => {
    bind("call_a", CHILD_A);
    vi.spyOn(useConnectionStore.getState(), "ensureSubscribe").mockResolvedValue(
      undefined,
    );
    const unsubscribe = vi.spyOn(
      useConnectionStore.getState(),
      "unsubscribeSession",
    );
    setDockviewApi({
      getPanel: (id: string) => (id === `agent-${CHILD_A}` ? {} : undefined),
    } as never);

    const roster = openRoster();
    const row = within(roster).getByRole("button", { name: "Subagent subagent" });
    fireEvent.click(row);
    fireEvent.click(row);

    expect(unsubscribe).not.toHaveBeenCalled();
  });
});

import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { BashJob, SubagentJob } from "../api/types";
import { useBashStore } from "../stores/bashStore";
import { setDockviewApi, useConnectionStore } from "../stores/connectionStore";
import { useEditorStore } from "../stores/editorStore";
import { useMessageStore } from "../stores/messageStore";
import { useSessionStore } from "../stores/sessionStore";
import { useSubagentStore } from "../stores/subagentStore";
import { emptySlice, useTurnStore, type TurnSlice } from "../stores/turnStore";
import {
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
  useSessionStore.setState({ project: null } as never);
  useEditorStore.setState({ openFile: originalOpenFile } as never);
});

afterEach(() => {
  cleanup();
  useBashStore.getState().reset();
  useSubagentStore.getState().reset();
  useMessageStore.setState({ bySession: new Map() });
  useTurnStore.setState({ byId: new Map() });
  useSessionStore.setState({ project: null } as never);
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

    // The count is hidden while collapsed and revealed on hover.
    const terminal = screen.getByTestId("capsule-terminal");
    expect(within(terminal).queryByText("×2")).toBeNull();
    fireEvent.mouseEnter(terminal);
    expect(within(terminal).getByText("×2")).toBeTruthy();
  });
});

describe("SessionStatusLine — hover expands the capsule inline", () => {
  it("collapses to icon-only and expands to label + count on hover, then restores", () => {
    useBashStore
      .getState()
      .applySnapshot("s1", { jobs: [bashJob, bashJob2], waits: [] });
    render(<SessionStatusLine sessionId="s1" />);

    const capsule = screen.getByTestId("capsule-terminal");
    // Collapsed: icon-only — no label, no count text in the DOM.
    expect(capsule.dataset.expanded).toBe("false");
    expect(within(capsule).queryByText("Terminals")).toBeNull();
    expect(within(capsule).queryByText("×2")).toBeNull();

    fireEvent.mouseEnter(capsule);
    expect(capsule.dataset.expanded).toBe("true");
    expect(within(capsule).getByText("Terminals")).toBeTruthy();
    expect(within(capsule).getByText("×2")).toBeTruthy();

    fireEvent.mouseLeave(capsule);
    expect(capsule.dataset.expanded).toBe("false");
    expect(within(capsule).queryByText("Terminals")).toBeNull();
    expect(within(capsule).queryByText("×2")).toBeNull();
  });

  it("expands only the hovered capsule (per-capsule state)", () => {
    render(<SessionStatusLine sessionId="s1" />);
    fireEvent.mouseEnter(screen.getByTestId("capsule-plan"));
    expect(
      within(screen.getByTestId("capsule-plan")).getByText("Plan"),
    ).toBeTruthy();
    expect(
      within(screen.getByTestId("capsule-todo")).queryByText("Tasks"),
    ).toBeNull();
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
    render(<SessionStatusLine sessionId="s1" />);
    fireEvent.click(screen.getByTestId("capsule-plan"));
    expect(screen.getByTestId("status-capsule-panel")).toBeTruthy();
    fireEvent.click(screen.getByTestId("capsule-plan"));
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
    render(<SessionStatusLine sessionId="s1" />);
    fireEvent.click(screen.getByTestId("capsule-todo"));
    expect(screen.getByTestId("status-capsule-panel")).toBeTruthy();
    fireEvent.mouseDown(document.body);
    expect(screen.queryByTestId("status-capsule-panel")).toBeNull();
  });

  it("closes the panel on Escape", () => {
    render(<SessionStatusLine sessionId="s1" />);
    fireEvent.click(screen.getByTestId("capsule-todo"));
    expect(screen.getByTestId("status-capsule-panel")).toBeTruthy();
    fireEvent.keyDown(document, { key: "Escape" });
    expect(screen.queryByTestId("status-capsule-panel")).toBeNull();
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
    render(<SessionStatusLine sessionId="s1" />);
    fireEvent.click(screen.getByTestId("capsule-todo"));
    const handle = screen.getByTestId("status-panel-resize");

    fireEvent.pointerDown(handle, { pointerId: 1, clientY: 400 });
    expect(document.body.style.cursor).toBe("ns-resize");

    // Esc closes the panel and unmounts the handle before any pointerup.
    fireEvent.keyDown(document, { key: "Escape" });
    expect(screen.queryByTestId("status-capsule-panel")).toBeNull();
    expect(document.body.style.cursor).toBe("");
    expect(document.body.style.userSelect).toBe("");
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
    fireEvent.click(screen.getByTestId("capsule-terminal"));
    expect(screen.getByText("No active terminals")).toBeTruthy();
    fireEvent.click(screen.getByTestId("capsule-subagent"));
    expect(screen.getByText("No subagents")).toBeTruthy();
    fireEvent.click(screen.getByTestId("capsule-todo"));
    expect(screen.getByText("No tasks yet")).toBeTruthy();
    fireEvent.click(screen.getByTestId("capsule-plan"));
    expect(screen.getByText("No active plan")).toBeTruthy();
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
    expect(screen.getByText("first")).toBeTruthy();
    expect(screen.getByText("third")).toBeTruthy();
    expect(screen.getByText("1/3")).toBeTruthy();
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

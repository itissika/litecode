import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { setDockviewApi, useConnectionStore } from "../stores/connectionStore";
import { openSubagentPanel } from "../lib/sessionPanelNav";
import { useMessageStore } from "../stores/messageStore";
import { useSessionStore } from "../stores/sessionStore";
import { useSubagentStore } from "../stores/subagentStore";
import { useTurnStore } from "../stores/turnStore";
import { SubagentRosterPanel } from "./SubagentRosterPanel";

// The roster is pure navigation now — the child transcript lives in its own
// dock panel. Mock the nav seam so a click is observable without a dockview.
vi.mock("../lib/sessionPanelNav", () => ({ openSubagentPanel: vi.fn() }));

const openSubagentPanelMock = vi.mocked(openSubagentPanel);

const PARENT = "s1";
const CHILD = "child-a";

function seedSession(agentId: string, preview: string, running = false): void {
  useSessionStore.setState({
    sessions: [
      {
        id: CHILD,
        project: "/p",
        updated_at: 0,
        preview,
        running,
        turn: null,
        agent_id: agentId,
        api_model_id: "m",
        parent_session_id: PARENT,
        parent_call_id: "call_a",
      },
    ],
  });
}

/**
 * A real P7 child row from `session/list`: parent ids set, its own agent_id,
 * and the two preview flavours the header has to choose between.
 */
function seedListSession(patch: {
  assistant_preview?: string;
  preview?: string;
}): void {
  useSessionStore.setState({
    sessions: [
      {
        id: CHILD,
        project: "/p",
        updated_at: 0,
        running: false,
        turn: null,
        agent_id: "researcher",
        api_model_id: "m",
        parent_session_id: PARENT,
        parent_call_id: "call_a",
        ...patch,
        preview: patch.preview ?? "user prompt",
      },
    ],
    byId: new Map(),
  });
}

function renderPanel(): HTMLElement {
  render(<SubagentRosterPanel sessionId={PARENT} />);
  return screen.getByTestId("subagent-roster");
}

beforeEach(() => {
  useConnectionStore.setState({ state: "connected" });
  useMessageStore.getState().onSubagentBound(PARENT, {
    session_id: PARENT,
    call_id: "call_a",
    child_session_id: CHILD,
  });
});

afterEach(() => {
  cleanup();
  setDockviewApi(null);
  useConnectionStore.setState({ state: "disconnected" });
  useMessageStore.getState().reset(PARENT);
  useMessageStore.getState().reset(CHILD);
  useTurnStore.getState().resetTurn(CHILD);
  useSubagentStore.getState().reset(PARENT);
  useSessionStore.setState({ sessions: [], byId: new Map() });
  openSubagentPanelMock.mockClear();
  vi.restoreAllMocks();
});

describe("SubagentRosterPanel — card header", () => {
  it("labels the child from the session list when the parent row is not loaded", () => {
    seedSession("researcher", "working on the wiring");
    const roster = renderPanel();

    expect(
      within(roster).getByTestId("subagent-roster-agent").textContent,
    ).toBe("researcher");
    expect(
      within(roster).getByRole("button", { name: "Subagent researcher" }),
    ).toBeTruthy();
  });

  it("says finished (not unknown) for an old launch whose row left the window", () => {
    // Durable binding + session/list entry, but the parent's launch/output rows
    // are outside the loaded window: the child terminated, the ok/error detail
    // is simply not reachable — that reads as "finished", never "unknown".
    seedListSession({});
    const roster = renderPanel();

    expect(
      within(roster).getByTestId("subagent-roster-finished").textContent,
    ).toBe("finished");
  });

  it("shows the child's last-message preview when the server sent no assistant text", () => {
    seedSession("researcher", "reading the panel wiring");
    const roster = renderPanel();

    expect(
      within(roster).getByTestId("subagent-roster-preview").textContent,
    ).toBe("reading the panel wiring");
    // The roster never embeds a transcript.
    expect(screen.queryByTestId("message-list")).toBeNull();
  });

  it("labels a never-subscribed child from the list alone (never 'subagent')", () => {
    // The P7 list payload: a real child row carrying its parent ids, its own
    // agent_id and the assistant preview. No subscription, no `byId`, no live
    // job and no parent launch row are available here.
    seedListSession({ assistant_preview: "read the panel wiring" });

    const roster = renderPanel();

    const label = within(roster).getByTestId("subagent-roster-agent").textContent;
    expect(label).toBe("researcher");
    expect(label).not.toBe("subagent");
    // Assistant text wins over the raw last-message preview.
    expect(
      within(roster).getByTestId("subagent-roster-preview").textContent,
    ).toBe("read the panel wiring");
  });

  it("falls back to `preview` when the child has no assistant text", () => {
    // The server omits `assistant_preview` when empty; here it sends "".
    seedListSession({ assistant_preview: "" });

    const roster = renderPanel();

    expect(
      within(roster).getByTestId("subagent-roster-preview").textContent,
    ).toBe("user prompt");
  });

  it("survives a reload: lists children from the session list alone (no bindings)", () => {
    // `agent/subagent_bound` is a live event that never replays, so after a
    // page reload bindings are empty — but the child row in `session/list`
    // (pushed on create, re-pulled on open) is durable. The roster's lifecycle
    // is the session's, not the binding event's.
    useMessageStore.getState().reset(PARENT);
    seedListSession({ assistant_preview: "worker summary" });

    const roster = renderPanel();

    expect(
      within(roster).getByRole("button", { name: "Subagent researcher" }),
    ).toBeTruthy();
    expect(
      within(roster).getByTestId("subagent-roster-preview").textContent,
    ).toBe("worker summary");
    expect(
      useMessageStore.getState().bySession.get(PARENT)?.subagentBindings,
    ).toEqual({});
  });

  it("uses the session's running flag when no live job is present", () => {
    seedSession("researcher", "busy", true);
    const roster = renderPanel();

    expect(
      within(roster).getByTestId("subagent-roster-running").textContent,
    ).toContain("running");
  });

  it("breathes the presence icon while the child is live", () => {
    seedSession("researcher", "busy", true);
    const { container } = render(<SubagentRosterPanel sessionId={PARENT} />);

    expect(container.querySelector(".tool-icon.sa-presence.sa-running")).toBeTruthy();
  });

  it("keeps card order stable when live session updates reorder the global list", () => {
    useMessageStore.getState().onSubagentBound(PARENT, {
      session_id: PARENT,
      call_id: "call_b",
      child_session_id: "child-b",
    });
    const session = (id: string, agent: string, updatedAt: number) => ({
      id,
      project: "/p",
      updated_at: updatedAt,
      preview: agent,
      running: true,
      turn: null,
      agent_id: agent,
      api_model_id: "m",
      parent_session_id: PARENT,
      parent_call_id: `call_${id.at(-1)}`,
    });
    useSessionStore.setState({
      sessions: [session(CHILD, "alpha", 2), session("child-b", "beta", 1)],
    });

    const roster = renderPanel();
    const headers = () => within(roster).getAllByRole("button");

    useSessionStore.setState({
      sessions: [session("child-b", "beta", 3), session(CHILD, "alpha", 2)],
    });

    expect(headers().map((header) => header.getAttribute("aria-label"))).toEqual([
      "Subagent alpha",
      "Subagent beta",
    ]);
  });

  it("falls back to the parent transcript row when the session is unknown", () => {
    useMessageStore.getState().onBufferItem(PARENT, {
      session_id: PARENT,
      seq: 0,
      kind: "item/tool_call",
      body: {
        type: "function_call",
        id: "fc_a",
        call_id: "call_a",
        name: "subagent_launch",
        arguments: JSON.stringify({ agent: "worker", prompt: "do it" }),
        status: "completed",
      },
    } as never);
    useMessageStore.getState().onBufferItem(PARENT, {
      session_id: PARENT,
      seq: 1,
      kind: "item/tool_result",
      body: {
        type: "function_call_output",
        call_id: "call_a",
        output: "done",
      },
    } as never);

    const roster = renderPanel();
    expect(
      within(roster).getByRole("button", { name: "Subagent worker" }),
    ).toBeTruthy();
    expect(
      within(roster).getByTestId("subagent-roster-finished").textContent,
    ).toBe("completed");
  });
});

describe("SubagentRosterPanel — row navigation", () => {
  it("opens the read-only subagent panel on click and embeds no transcript", () => {
    seedSession("researcher", "busy", true);
    const roster = renderPanel();

    // No inline MessageList; the roster only lists metadata.
    expect(screen.queryByTestId("message-list")).toBeNull();

    fireEvent.click(
      within(roster).getByRole("button", { name: "Subagent researcher" }),
    );

    expect(openSubagentPanelMock).toHaveBeenCalledWith(CHILD);
    // Still no embedded transcript after the click.
    expect(screen.queryByTestId("message-list")).toBeNull();
  });

  it("does not own a child subscription", () => {
    seedSession("researcher", "busy", true);
    const subscribe = vi
      .spyOn(useConnectionStore.getState(), "ensureSubscribe")
      .mockResolvedValue(undefined);

    const roster = renderPanel();
    fireEvent.click(
      within(roster).getByRole("button", { name: "Subagent researcher" }),
    );

    expect(subscribe).not.toHaveBeenCalledWith(CHILD);
  });
});

describe("SubagentRosterPanel — session list refresh (P7)", () => {
  it("re-pulls session/list when the panel opens", () => {
    const listSessions = vi
      .spyOn(useSessionStore.getState(), "listSessions")
      .mockImplementation(() => {});

    renderPanel();

    expect(listSessions).toHaveBeenCalledTimes(1);
  });

  it("does not race the socket handshake while disconnected", () => {
    useConnectionStore.setState({ state: "disconnected" });
    const listSessions = vi
      .spyOn(useSessionStore.getState(), "listSessions")
      .mockImplementation(() => {});

    renderPanel();

    expect(listSessions).not.toHaveBeenCalled();
  });
});

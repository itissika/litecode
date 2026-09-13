import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { userTextItem } from "../api/adapter";
import type { HumanRow } from "../api/types";
import { setDockviewApi, useConnectionStore } from "../stores/connectionStore";
import { useMessageStore } from "../stores/messageStore";
import { useSessionStore } from "../stores/sessionStore";
import { useSubagentStore } from "../stores/subagentStore";
import { useTurnStore } from "../stores/turnStore";
import { SubagentRosterPanel } from "./SubagentRosterPanel";
import {
  isSubagentRosterHeld,
  resetSubagentRosterHolds,
} from "./subagentRosterHolds";

/**
 * The expanded card mounts the REAL MessageList (virtualizer), so render every
 * virtual item — jsdom has no layout and would otherwise measure 0 items.
 */
vi.mock("@tanstack/react-virtual", () => ({
  useVirtualizer: ({
    count,
    getItemKey,
  }: {
    count: number;
    getItemKey?: (index: number) => string | number;
  }) => ({
    getVirtualItems: () =>
      Array.from({ length: count }, (_, index) => ({
        key: getItemKey?.(index) ?? index,
        index,
        start: index * 200,
        size: 200,
        end: (index + 1) * 200,
      })),
    getTotalSize: () => count * 200,
    measureElement: () => {},
    scrollToEnd: () => {},
    scrollToIndex: () => {},
    isAtEnd: () => true,
    options: {},
  }),
}));

class ResizeObserverStub {
  observe() {}
  unobserve() {}
  disconnect() {}
}
vi.stubGlobal("ResizeObserver", ResizeObserverStub);

const PARENT = "s1";
const CHILD = "child-a";

const userRow = (seq: number, text: string): HumanRow => ({
  seq,
  kind: "item/user",
  body: userTextItem(text),
});

const assistantRow = (seq: number, text: string): HumanRow => ({
  seq,
  kind: "item/assistant",
  body: {
    type: "message",
    role: "assistant",
    id: `msg-${seq}`,
    status: "completed",
    content: [{ type: "output_text", text, annotations: [] }],
  },
});

function seedChild(rows: HumanRow[]): void {
  useMessageStore.setState((s) => {
    const bySession = new Map(s.bySession);
    bySession.set(CHILD, {
      bySeq: new Map(rows.map((r) => [r.seq, r])),
      messages: rows,
      display: rows,
      pendingUser: null,
      fromSeq: 0,
      toSeq: rows.length,
      userDetailBefore: 0,
      loadingHistory: false,
      hydrated: true,
      shapeError: null,
      subagentBindings: {},
      blockLogGrowth: false,
      turnEndNotice: null,
      itemIdToSeq: new Map(),
    });
    return { bySession };
  });
}

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
  resetSubagentRosterHolds();
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

  it("labels the child from byId.activePrimary when the list has no entry yet", () => {
    // Fallback path: the list RPC has not landed (or the child is not in the
    // snapshot), so `byId` — hydrated from the child's own snapshot push on
    // subscribe — is all the panel has.
    useSessionStore.getState().applySnapshot({
      session_id: CHILD,
      project: "/p",
      agent_id: "researcher",
      api_model_id: "m",
      buffer: { last_seq: 0, next_seq: 0, revision: 0 },
      turn: null,
    });
    expect(useSessionStore.getState().sessions).toHaveLength(0);

    const roster = renderPanel();

    expect(
      within(roster).getByTestId("subagent-roster-agent").textContent,
    ).toBe("researcher");
    expect(
      within(roster).getByRole("button", { name: "Subagent researcher" }),
    ).toBeTruthy();
  });

  it("shows the child's last-message preview when the server sent no assistant text", () => {
    seedSession("researcher", "reading the panel wiring");
    const roster = renderPanel();

    expect(
      within(roster).getByTestId("subagent-roster-preview").textContent,
    ).toBe("reading the panel wiring");
    // Collapsed: no transcript mounted yet.
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

describe("SubagentRosterPanel — expanded card is the full child transcript", () => {
  it("subscribes on expand and renders one bubble per child user row", () => {
    seedSession("researcher", "busy", true);
    seedChild([
      userRow(0, "do the thing"),
      assistantRow(1, "done"),
      userRow(2, "now do this"),
    ]);
    const subscribe = vi
      .spyOn(useConnectionStore.getState(), "ensureSubscribe")
      .mockResolvedValue(undefined);

    const roster = renderPanel();
    fireEvent.click(within(roster).getByRole("button", { name: /Subagent/ }));

    expect(subscribe).toHaveBeenCalledWith(CHILD);
    // P5-4: every item/user row of the child renders as a user bubble — the
    // launch prompt AND the following subagent_send message.
    expect(screen.getByTestId("message-list")).toBeTruthy();
    expect(
      document.querySelectorAll("[data-user-message-bubble]"),
    ).toHaveLength(2);
    expect(screen.getByText("now do this")).toBeTruthy();
  });

  it("unsubscribes on collapse but KEEPS the child slices (P6 incremental re-expand)", () => {
    seedSession("researcher", "busy", true);
    seedChild([userRow(0, "do the thing")]);
    vi.spyOn(useConnectionStore.getState(), "ensureSubscribe").mockResolvedValue(
      undefined,
    );
    const unsubscribe = vi.spyOn(
      useConnectionStore.getState(),
      "unsubscribeSession",
    );

    const roster = renderPanel();
    const row = within(roster).getByRole("button", { name: /Subagent/ });
    fireEvent.click(row);
    fireEvent.click(row);

    expect(unsubscribe).toHaveBeenCalledWith(CHILD);
    expect(
      useMessageStore.getState().bySession.get(CHILD)?.messages.length ?? 0,
    ).toBe(1);
  });

  it("leaves the subscription alone while the child owns a dock tab", () => {
    seedSession("researcher", "busy", true);
    seedChild([userRow(0, "do the thing")]);
    vi.spyOn(useConnectionStore.getState(), "ensureSubscribe").mockResolvedValue(
      undefined,
    );
    const unsubscribe = vi.spyOn(
      useConnectionStore.getState(),
      "unsubscribeSession",
    );
    setDockviewApi({
      getPanel: (id: string) => (id === `agent-${CHILD}` ? {} : undefined),
    } as never);

    const roster = renderPanel();
    const row = within(roster).getByRole("button", { name: /Subagent/ });
    fireEvent.click(row);
    fireEvent.click(row);

    expect(unsubscribe).not.toHaveBeenCalled();
  });

  it("holds the child in the roster registry while expanded (tab-close guard)", () => {
    seedSession("researcher", "busy", true);
    seedChild([userRow(0, "do the thing")]);
    vi.spyOn(useConnectionStore.getState(), "ensureSubscribe").mockResolvedValue(
      undefined,
    );

    const roster = renderPanel();
    const row = within(roster).getByRole("button", { name: /Subagent/ });
    fireEvent.click(row);
    expect(isSubagentRosterHeld(CHILD)).toBe(true);

    fireEvent.click(row);
    expect(isSubagentRosterHeld(CHILD)).toBe(false);
  });

  it("re-expands onto the retained slices: re-subscribes, no cold start", () => {
    seedSession("researcher", "busy", false);
    seedChild([userRow(0, "do the thing")]);
    const subscribe = vi
      .spyOn(useConnectionStore.getState(), "ensureSubscribe")
      .mockResolvedValue(undefined);

    const roster = renderPanel();
    const row = within(roster).getByRole("button", { name: /Subagent/ });
    fireEvent.click(row);
    fireEvent.click(row);

    // Collapsed: the projection is retained (P6), not reset.
    expect(
      useMessageStore.getState().bySession.get(CHILD)?.messages.length ?? 0,
    ).toBe(1);

    fireEvent.click(row);

    expect(subscribe).toHaveBeenCalledTimes(2);
    expect(screen.getByText("do the thing")).toBeTruthy();
    expect(
      useMessageStore.getState().bySession.get(CHILD)?.messages.length ?? 0,
    ).toBe(1);
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

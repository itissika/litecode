import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import type { IDockviewPanelProps } from "dockview-react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { HumanRow } from "../../api/types";
import { userTextItem } from "../../api/adapter";
import { releaseSessionTab } from "../../components/sessionTeardown";
import { openSessionPanel } from "../../lib/sessionPanelNav";
import { useConnectionStore } from "../../stores/connectionStore";
import { useMessageStore } from "../../stores/messageStore";
import { useSessionStore } from "../../stores/sessionStore";
import { useToastStore } from "../../stores/toastStore";
import { useTurnStore } from "../../stores/turnStore";
import { SubagentReadOnlyPanel } from "./SubagentReadOnlyPanel";

// The panel only needs the nav seam for the root escape hatch; keep the rest of
// the module (pending-reveal) real so the transcript reveal contract is intact.
vi.mock("../../lib/sessionPanelNav", async (orig) => ({
  ...(await (orig as () => Promise<object>)()),
  openSessionPanel: vi.fn(),
}));
vi.mock("../../components/sessionTeardown", () => ({
  releaseSessionTab: vi.fn(),
}));

// Render every virtual item — jsdom has no layout.
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

const releaseSessionTabMock = vi.mocked(releaseSessionTab);
const openSessionPanelMock = vi.mocked(openSessionPanel);

const CHILD = "child-a";

const userRow = (seq: number, text: string): HumanRow => ({
  seq,
  kind: "item/user",
  state: "final",
  body: userTextItem(text),
});

const assistantRow = (seq: number, text: string): HumanRow => ({
  seq,
  kind: "item/assistant",
  state: "final",
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
    });
    return { bySession };
  });
}

function seedSession(parentSessionId: string | null): void {
  useSessionStore.setState({
    sessions: [
      {
        id: CHILD,
        project: "/p",
        updated_at: 0,
        preview: "the child summary",
        running: false,
        turn: null,
        agent_id: "researcher",
        api_model_id: "m",
        parent_session_id: parentSessionId,
        parent_call_id: parentSessionId ? "call_a" : null,
      },
    ],
  });
}

interface PanelApiStub {
  isActive: boolean;
  onDidActiveChange: () => { dispose: () => void };
  close: ReturnType<typeof vi.fn>;
  setTitle: ReturnType<typeof vi.fn>;
}

function panelProps(sessionId: string): {
  params: { sessionId: string };
  api: PanelApiStub;
} {
  return {
    params: { sessionId },
    api: {
      isActive: true,
      onDidActiveChange: () => ({ dispose: () => {} }),
      close: vi.fn(),
      setTitle: vi.fn(),
    },
  };
}

/** api.close is stubbed on a per-render basis; expose the mock for assertions. */
function renderPanel(sessionId = CHILD) {
  const props = panelProps(sessionId);
  const view = render(
    <SubagentReadOnlyPanel {...(props as unknown as IDockviewPanelProps)} />,
  );
  return { view, api: props.api };
}

beforeEach(() => {
  useConnectionStore.setState({
    state: "connected",
    ensureSubscribe: vi.fn(async () => {}) as never,
  });
});

afterEach(() => {
  cleanup();
  useConnectionStore.setState({ state: "disconnected" });
  useMessageStore.getState().reset(CHILD);
  useTurnStore.getState().resetTurn(CHILD);
  useSessionStore.setState({ sessions: [], byId: new Map() });
  releaseSessionTabMock.mockClear();
  openSessionPanelMock.mockClear();
  vi.restoreAllMocks();
  vi.useRealTimers();
});

describe("SubagentReadOnlyPanel — full read-only transcript", () => {
  it("renders the child transcript with the derived subagent controls, no composer", () => {
    seedSession("root");
    seedChild([
      userRow(0, "do the thing"),
      assistantRow(1, "done"),
      userRow(2, "now do this"),
    ]);

    renderPanel();

    // Full transcript is present.
    expect(screen.getByTestId("message-list")).toBeTruthy();
    expect(screen.getByText("now do this")).toBeTruthy();
    expect(
      document.querySelectorAll("[data-user-message-bubble]"),
    ).toHaveLength(2);

    // Derived subagent view: the status capsules + the session-row knobs are
    // mounted (model / tier / context mode are honored by the child's own next
    // turn), with the Workers capsule and the composer gone.
    expect(screen.getByTestId("session-status-line")).toBeTruthy();
    expect(screen.getByTestId("capsule-terminal")).toBeTruthy();
    expect(screen.queryByTestId("capsule-todo")).toBeNull();
    expect(screen.queryByTestId("capsule-plan")).toBeNull();
    expect(screen.queryByTestId("capsule-subagent")).toBeNull();
    expect(screen.getByTestId("subagent-controls")).toBeTruthy();
    expect(screen.getByTitle("Context usage")).toBeTruthy();

    // No human-composition surface at all.
    expect(screen.queryByTestId("chat-input")).toBeNull();
    expect(screen.queryByTestId("permission-card")).toBeNull();
    expect(screen.queryByTestId("mini-chat-input")).toBeNull();
    expect(document.querySelector("textarea")).toBeNull();
    expect(screen.queryByTitle("Send")).toBeNull();
    expect(screen.queryByTitle("Cancel")).toBeNull();
  });

  it("offers the knobs but none of the human-owned write actions", () => {
    seedSession("root");
    seedChild([userRow(0, "hi")]);

    renderPanel();

    // The ring reports usage; compaction stays a primary-panel action.
    fireEvent.click(screen.getByTitle("Context usage"));
    expect(screen.getByText("No context usage yet")).toBeTruthy();
    expect(screen.queryByLabelText("Compact context")).toBeNull();
  });

  it("connected: ensures the child subscription", () => {
    seedSession("root");
    seedChild([userRow(0, "hi")]);
    const subscribe = vi
      .spyOn(useConnectionStore.getState(), "ensureSubscribe")
      .mockResolvedValue(undefined);

    renderPanel();

    expect(subscribe).toHaveBeenCalledWith(CHILD);
  });

  it("closes and toasts when the session no longer exists", async () => {
    seedSession("root");
    seedChild([userRow(0, "hi")]);
    const close = vi.fn();
    vi.spyOn(
      useConnectionStore.getState(),
      "ensureSubscribe",
    ).mockRejectedValue(new Error("session not found"));
    const showToast = vi
      .spyOn(useToastStore.getState(), "showToast")
      .mockImplementation(() => {});

    const props = panelProps(CHILD);
    props.api.close = close;
    render(
      <SubagentReadOnlyPanel {...(props as unknown as IDockviewPanelProps)} />,
    );

    await waitFor(() => {
      expect(showToast).toHaveBeenCalled();
      expect(close).toHaveBeenCalled();
    });
  });

  it("releases the session tab on unmount", () => {
    seedSession("root");
    seedChild([userRow(0, "hi")]);

    const { view } = renderPanel();
    view.unmount();

    expect(releaseSessionTabMock).toHaveBeenCalledWith(CHILD);
  });

  it("clicking a user bubble never opens MiniChat nor replays", () => {
    seedSession("root");
    seedChild([userRow(0, "do the thing"), assistantRow(1, "done")]);
    const replay = vi.spyOn(useTurnStore.getState(), "replayFromAnchor");

    renderPanel();
    const bubble = document.querySelector("[data-user-message-bubble]");
    expect(bubble).toBeTruthy();
    // Read-only: no text cursor advertising an edit affordance.
    expect(bubble?.className ?? "").not.toContain("cursor-text");

    fireEvent.click(bubble as Element);

    expect(screen.queryByTestId("mini-chat-input")).toBeNull();
    expect(replay).not.toHaveBeenCalled();
  });

  it("updates the tab title from the session preview", () => {
    seedSession("root");
    seedChild([userRow(0, "hi")]);
    const props = panelProps(CHILD);
    render(
      <SubagentReadOnlyPanel {...(props as unknown as IDockviewPanelProps)} />,
    );

    expect(props.api.setTitle).toHaveBeenCalledWith("the child summary");
  });
});

describe("SubagentReadOnlyPanel — root escape hatch", () => {
  it("offers an explicit writable-session button when the id is a confirmed root", () => {
    // The id was routed here while unknown; the list now proves it is a root.
    // Stay read-only (safe) but never trap the user.
    seedSession(null);
    seedChild([userRow(0, "hi")]);

    renderPanel();

    expect(screen.getByTestId("open-writable-session")).toBeTruthy();
    expect(screen.queryByTestId("chat-input")).toBeNull();
  });

  it("closing then opening the writable panel avoids a concurrent double host", () => {
    vi.useFakeTimers();
    seedSession(null);
    seedChild([userRow(0, "hi")]);
    const { api } = renderPanel();

    fireEvent.click(screen.getByTestId("open-writable-session"));

    // Current panel closes immediately (its unmount releases the subscription)…
    expect(api.close).toHaveBeenCalled();
    // …and the writable panel is only opened afterwards.
    expect(openSessionPanelMock).not.toHaveBeenCalled();
    vi.runAllTimers();
    expect(openSessionPanelMock).toHaveBeenCalledWith(CHILD);
  });

  it("does not offer the escape hatch for a child session", () => {
    seedSession("root");
    seedChild([userRow(0, "hi")]);

    renderPanel();

    expect(screen.queryByTestId("open-writable-session")).toBeNull();
  });
});

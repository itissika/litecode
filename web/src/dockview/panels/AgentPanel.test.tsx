import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import type { IDockviewPanelProps } from "dockview-react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { userTextItem } from "../../api/adapter";
import type { HumanRow } from "../../api/types";
import { useConnectionStore } from "../../stores/connectionStore";
import { useMessageStore } from "../../stores/messageStore";
import { useSessionStore } from "../../stores/sessionStore";
import { useTurnStore } from "../../stores/turnStore";
import { AgentPanel } from "./AgentPanel";

// Keep the writable surface cheap and observable.
vi.mock("../../components/AgentChatInput", () => ({
  AgentChatInput: () => <div data-testid="chat-input" />,
}));
vi.mock("../../components/SessionStatusLine", () => ({
  SessionStatusLine: () => <div data-testid="session-status-line" />,
}));
vi.mock("../../components/PermissionModal", () => ({
  PermissionCard: () => <div data-testid="permission-card" />,
}));
vi.mock("../../components/sessionTeardown", () => ({
  releaseSessionTab: vi.fn(),
}));

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

const ID = "session-1";

const userRow = (seq: number, text: string): HumanRow => ({
  seq,
  kind: "item/user",
  body: userTextItem(text),
});

function seedMessages(rows: HumanRow[]): void {
  useMessageStore.setState((s) => {
    const bySession = new Map(s.bySession);
    bySession.set(ID, {
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

function seedSession(parentSessionId: string | null | undefined): void {
  if (parentSessionId === undefined) {
    useSessionStore.setState({ sessions: [], byId: new Map() });
    return;
  }
  useSessionStore.setState({
    sessions: [
      {
        id: ID,
        project: "/p",
        updated_at: 0,
        preview: "preview",
        running: false,
        turn: null,
        agent_id: "default",
        api_model_id: "m",
        parent_session_id: parentSessionId,
        parent_call_id: parentSessionId ? "call_a" : null,
      },
    ],
  });
}

function panelProps(
  sessionId: string,
  extraParams?: Record<string, unknown>,
): IDockviewPanelProps {
  return {
    params: { sessionId, ...extraParams },
    api: {
      isActive: true,
      onDidActiveChange: () => ({ dispose: () => {} }),
      close: vi.fn(),
      setTitle: vi.fn(),
    },
  } as unknown as IDockviewPanelProps;
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
  useMessageStore.getState().reset(ID);
  useTurnStore.getState().resetTurn(ID);
  useSessionStore.setState({ sessions: [], byId: new Map() });
  vi.restoreAllMocks();
});

describe("AgentPanel — fail-closed identity classification", () => {
  it("renders the writable AgentChatShell only for a confirmed root", () => {
    seedSession(null);
    seedMessages([userRow(0, "hello")]);

    render(<AgentPanel {...panelProps(ID)} />);

    expect(screen.getByTestId("chat-input")).toBeTruthy();
    expect(screen.getByTestId("session-status-line")).toBeTruthy();
  });

  it("renders read-only (no Composer) for a known child", () => {
    seedSession("root");
    seedMessages([userRow(0, "hello")]);

    render(<AgentPanel {...panelProps(ID)} />);

    expect(screen.getByTestId("message-list")).toBeTruthy();
    expect(screen.queryByTestId("chat-input")).toBeNull();
    expect(screen.queryByTestId("session-status-line")).toBeNull();
    expect(document.querySelector("textarea")).toBeNull();
  });

  it("fails closed to read-only while the session identity is unknown", () => {
    // Not in the session list yet (list still loading / never seen).
    seedSession(undefined);
    seedMessages([userRow(0, "hello"), userRow(1, "again")]);

    render(<AgentPanel {...panelProps(ID)} />);

    // Transcript is shown read-only; never the Composer.
    expect(screen.getByTestId("message-list")).toBeTruthy();
    expect(screen.queryByTestId("chat-input")).toBeNull();
  });

  it("read-only content really disables the user bubble edit affordance", () => {
    seedSession(undefined);
    seedMessages([userRow(0, "hello")]);
    const replay = vi.spyOn(useTurnStore.getState(), "replayFromAnchor");

    render(<AgentPanel {...panelProps(ID)} />);

    const bubble = document.querySelector("[data-user-message-bubble]");
    expect(bubble).toBeTruthy();
    expect(bubble?.className ?? "").not.toContain("cursor-text");
    fireEvent.click(bubble as Element);
    expect(screen.queryByTestId("mini-chat-input")).toBeNull();
    expect(replay).not.toHaveBeenCalled();
  });

  it("switches to the writable shell once the list confirms the session is a root", () => {
    seedSession(undefined);
    seedMessages([userRow(0, "hello")]);

    const view = render(<AgentPanel {...panelProps(ID)} />);
    expect(screen.queryByTestId("chat-input")).toBeNull();

    // The list arrives and classifies the id as a root.
    seedSession(null);
    view.rerender(<AgentPanel {...panelProps(ID)} />);

    expect(screen.getByTestId("chat-input")).toBeTruthy();
  });

  it("renders the writable shell immediately for a freshly-created root (trusted provenance, absent from the list)", () => {
    // `newSession()` → `openSessionPanel(newId)`: the new root is not in
    // `session/list` yet, so absent=unknown would otherwise blank it. The trusted
    // entry tags params with `sessionKind: "root"`.
    seedSession(undefined);
    seedMessages([]);

    render(<AgentPanel {...panelProps(ID, { sessionKind: "root" })} />);

    expect(screen.getByTestId("chat-input")).toBeTruthy();
    expect(screen.getByTestId("session-status-line")).toBeTruthy();
  });

  it("stays read-only for an unknown id without provenance", () => {
    seedSession(undefined);
    seedMessages([userRow(0, "hello")]);

    render(<AgentPanel {...panelProps(ID)} />);

    expect(screen.getByTestId("message-list")).toBeTruthy();
    expect(screen.queryByTestId("chat-input")).toBeNull();
  });

  it("stays read-only for a known child even if a legacy panel carries no provenance", () => {
    seedSession("root");
    seedMessages([userRow(0, "hello")]);

    render(<AgentPanel {...panelProps(ID)} />);

    expect(screen.queryByTestId("chat-input")).toBeNull();
  });

  it("keeps a known child read-only even with stale or forged root provenance", () => {
    // Durable parent metadata is authoritative once present. A damaged/restored
    // layout param must never upgrade a child session to the writable surface.
    seedSession("root");
    seedMessages([userRow(0, "hello")]);

    render(<AgentPanel {...panelProps(ID, { sessionKind: "root" })} />);

    expect(screen.queryByTestId("chat-input")).toBeNull();
    expect(screen.getByTestId("message-list")).toBeTruthy();
  });
});

/**
 * Sidebar session list. The server lists subagent CHILD sessions too (the roster
 * needs them to label a never-subscribed child), so the sidebar must show roots
 * only — otherwise every launched worker appears as a session of its own.
 */
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { SessionInfo } from "../api/types";
import { useConnectionStore } from "../stores/connectionStore";
import { useSessionStore } from "../stores/sessionStore";
import { useUnreadStore } from "../stores/unreadStore";
import { bindDockview } from "../dockview/workbench/host";
import { SessionList } from "./SessionList";

function session(id: string, patch: Partial<SessionInfo> = {}): SessionInfo {
  return {
    id,
    project: "/p",
    updated_at: 0,
    preview: id,
    running: false,
    turn: null,
    agent_id: "default",
    api_model_id: "m",
    ...patch,
  };
}

beforeEach(() => {
  window.localStorage.clear();
  useUnreadStore.setState({ unread: {} });
  useSessionStore.setState({
    sessions: [],
    sessionsLoading: false,
    sessionListError: null,
  } as never);
});

afterEach(() => {
  cleanup();
  bindDockview(null);
  useUnreadStore.setState({ unread: {} });
  useSessionStore.setState({
    sessions: [],
    sessionsLoading: false,
    sessionListError: null,
  } as never);
  useConnectionStore.setState({ state: "disconnected" } as never);
  vi.restoreAllMocks();
});

describe("SessionList — folders and titles", () => {
  it("groups roots by folder, keeps empty folders, and shows ungrouped", async () => {
    const filed = session("s-filed", {
      preview: "filed work",
      folder_id: "f1",
    });
    const loose = session("s-loose", { preview: "loose work" });
    useConnectionStore.setState({
      state: "connected",
      sendRpc: vi.fn(async () => ({
        sessions: [filed, loose],
        folders: [
          { id: "f1", name: "Work", sort_order: 0 },
          { id: "f2", name: "Archive", sort_order: 1 },
        ],
      })),
    } as never);

    render(<SessionList />);

    expect(await screen.findByText("Work")).toBeTruthy();
    expect(screen.getByText("Archive")).toBeTruthy();
    expect(screen.getByText("filed work")).toBeTruthy();
    expect(screen.getByText("loose work")).toBeTruthy();
    expect(screen.getByText("Ungrouped")).toBeTruthy();
    expect(screen.getByText("Empty")).toBeTruthy();
  });

  it("orders folders by sort_order, not arrival order", async () => {
    const first = session("s1", { preview: "one" });
    useConnectionStore.setState({
      state: "connected",
      sendRpc: vi.fn(async () => ({
        sessions: [first],
        folders: [
          { id: "f2", name: "Second", sort_order: 2 },
          { id: "f1", name: "First", sort_order: 1 },
        ],
      })),
    } as never);

    render(<SessionList />);
    await screen.findByText("Second");
    const headers = screen.getAllByRole("button", {
      name: /Collapse folder: (Second|First)/,
    });
    expect(headers[0]?.textContent).toContain("First");
    expect(headers[1]?.textContent).toContain("Second");
  });

  it("shows the title override instead of the auto preview", async () => {
    const titled = session("s1", {
      preview: "auto preview",
      title: "Release checklist",
    });
    useConnectionStore.setState({
      state: "connected",
      sendRpc: vi.fn(async () => ({ sessions: [titled], folders: [] })),
    } as never);

    render(<SessionList />);

    expect(await screen.findByText("Release checklist")).toBeTruthy();
    expect(screen.queryByText("auto preview")).toBeNull();
  });

  it("starts renaming on double-click", async () => {
    const target = session("s1", { preview: "auto preview" });
    useConnectionStore.setState({
      state: "connected",
      sendRpc: vi.fn(async () => ({ sessions: [target], folders: [] })),
    } as never);

    render(<SessionList />);
    fireEvent.doubleClick(await screen.findByText("auto preview"));

    expect(
      screen.getByRole("textbox", { name: "Session title" }),
    ).toBeTruthy();
  });

  it("renames a folder inline without toggling its collapse state", async () => {
    const sendRpc = vi.fn(async (method: string) => {
      if (method === "session/list") {
        return {
          sessions: [],
          folders: [{ id: "f1", name: "Work", sort_order: 0 }],
        };
      }
      return {};
    });
    useConnectionStore.setState({ state: "connected", sendRpc } as never);

    render(<SessionList />);
    const header = await screen.findByRole("button", {
      name: "Collapse folder: Work",
    });
    fireEvent.contextMenu(header);
    fireEvent.click(await screen.findByRole("menuitem", { name: "Rename folder" }));

    const input = screen.getByRole("textbox", { name: "Folder name" });
    fireEvent.change(input, { target: { value: "Plan" } });
    fireEvent.keyDown(input, { key: "Enter" });

    expect(sendRpc).toHaveBeenCalledWith("session/folder/rename", {
      id: "f1",
      name: "Plan",
    });
    const renamed = await screen.findByRole("button", {
      name: "Collapse folder: Plan",
    });
    expect(renamed.getAttribute("aria-expanded")).toBe("true");
  });

  it("renames a session from the context menu", async () => {
    const target = session("s1", { preview: "auto preview" });
    const sendRpc = vi.fn(async (method: string) => {
      if (method === "session/list") return { sessions: [target], folders: [] };
      return {};
    });
    useConnectionStore.setState({ state: "connected", sendRpc } as never);

    render(<SessionList />);
    fireEvent.contextMenu(await screen.findByText("auto preview"));
    fireEvent.click(await screen.findByRole("menuitem", { name: "Rename session" }));

    const input = screen.getByRole("textbox", { name: "Session title" });
    fireEvent.change(input, { target: { value: "Renamed" } });
    fireEvent.keyDown(input, { key: "Enter" });

    expect(sendRpc).toHaveBeenCalledWith("session/rename", {
      id: "s1",
      title: "Renamed",
    });
    expect(await screen.findByText("Renamed")).toBeTruthy();
  });
});

describe("SessionList — active session marker", () => {
  it("marks the session the agent area shows, and only that one", async () => {
    const shown = session("s-shown", { preview: "shown work" });
    const hidden = session("s-hidden", { preview: "hidden work" });
    useConnectionStore.setState({
      state: "connected",
      sendRpc: vi.fn(async () => ({ sessions: [shown, hidden], folders: [] })),
    } as never);

    const activePanel = {
      id: "agent-s-shown",
      params: { sessionId: "s-shown" },
      api: { component: "agent" },
    };
    bindDockview({
      groups: [
        {
          api: { id: "center", location: { type: "grid" }, isVisible: true },
          activePanel,
          panels: [activePanel],
        },
        {
          api: { id: "sessions", location: { type: "edge" }, isVisible: true },
          activePanel: {
            id: "sessions",
            params: {},
            api: { component: "sessions" },
          },
          panels: [],
        },
      ],
      getPanel: () => undefined,
      onDidLayoutChange: () => ({ dispose: () => {} }),
    } as never);

    render(<SessionList />);

    const row = (await screen.findByText("shown work")).closest(
      "[data-session-active]",
    );
    expect(row?.getAttribute("data-session-active")).toBe("true");
    expect(
      screen.getByText("hidden work").closest("[data-session-active]"),
    ).toBeNull();
  });
});

describe("SessionList — unread marker", () => {
  it("shows a dot until the mark is consumed", async () => {
    const target = session("s1", { preview: "finished work" });
    useConnectionStore.setState({
      state: "connected",
      sendRpc: vi.fn(async () => ({ sessions: [target], folders: [] })),
    } as never);

    render(<SessionList />);
    await screen.findByText("finished work");
    expect(
      document.querySelector('[data-session-unread="true"]'),
    ).toBeNull();

    act(() => useUnreadStore.getState().mark("s1"));
    expect(
      document.querySelector('[data-session-unread="true"]'),
    ).toBeTruthy();

    act(() => useUnreadStore.getState().consume("s1"));
    expect(
      document.querySelector('[data-session-unread="true"]'),
    ).toBeNull();
  });
});

describe("SessionList — root filtering (P7)", () => {
  it("renders roots only while the store still carries the children", async () => {
    const root = session("root-1", { preview: "root work" });
    const child = session("child-1", {
      preview: "child work",
      parent_session_id: "root-1",
      parent_call_id: "call_a",
    });
    useConnectionStore.setState({
      state: "connected",
      sendRpc: vi.fn(async () => ({ sessions: [root, child] })),
    } as never);

    render(<SessionList />);

    expect(await screen.findByText("root work")).toBeTruthy();
    // The child is in the store — the roster reads it from there — but the
    // sidebar must not surface it as a session of its own.
    expect(useSessionStore.getState().sessions).toHaveLength(2);
    expect(screen.queryByText("child work")).toBeNull();
    expect(
      screen.getAllByRole("button", { name: "Delete session" }),
    ).toHaveLength(1);
    // The header count is gone; the workspace usage read-out took its place at
    // the far right.
    expect(screen.getByRole("button", { name: "Workspace usage" })).toBeTruthy();
  });

  it("never lists a child that arrived on the lifecycle feed first", async () => {
    const root = session("root-1", { preview: "root work" });
    useConnectionStore.setState({
      state: "connected",
      sendRpc: vi.fn(async () => ({ sessions: [root] })),
    } as never);

    render(<SessionList />);
    expect(await screen.findByText("root work")).toBeTruthy();

    // A child's `created` broadcast can beat the (racy) list push to the client.
    act(() => {
      useSessionStore.getState().onSessionLifecycle({
        session_id: "child-1",
        event: "created",
        project: "/p",
        agent_id: "researcher",
        parent_session_id: "root-1",
        parent_call_id: "call_a",
        updated_at: 2,
        turn: null,
      });
    });

    expect(
      useSessionStore
        .getState()
        .sessions.map((s) => s.id)
        .sort(),
    ).toEqual(["child-1", "root-1"]);
    expect(
      screen.getAllByRole("button", { name: "Delete session" }),
    ).toHaveLength(1);
  });

  it("still shows an empty sidebar when the list carries children only", async () => {
    const child = session("child-1", {
      preview: "child work",
      parent_session_id: "root-1",
    });
    useConnectionStore.setState({
      state: "connected",
      sendRpc: vi.fn(async () => ({ sessions: [child] })),
    } as never);

    render(<SessionList />);

    expect(await screen.findByText("No saved sessions yet.")).toBeTruthy();
  });
});

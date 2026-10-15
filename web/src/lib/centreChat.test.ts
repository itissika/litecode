import type { DockviewApi } from "dockview-react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { bindDockview } from "../dockview/workbench/host";
import { useSessionStore } from "../stores/sessionStore";
import {
  noteLayoutSettled,
  noteTransportReady,
  resetCentreChatForTests,
} from "./centreChat";
import {
  noteRootAttached,
  noteSessionListSettled,
  whenCentreSettled,
} from "./centreSettle";

/** Minimal dockview stand-in: the check only reads `panels[].api`. */
function fakeApi(
  panels: { component: string; where: string }[],
): DockviewApi {
  return {
    panels: panels.map((panel) => ({
      api: { component: panel.component, location: { type: panel.where } },
    })),
  } as unknown as DockviewApi;
}

const empty = fakeApi([]);
const railsOnly = fakeApi([
  { component: "filetree", where: "left" },
  { component: "sessions", where: "right" },
  { component: "terminal", where: "bottom" },
]);

const originalNewSession = useSessionStore.getState().newSession;
let newSession: ReturnType<typeof vi.fn>;

beforeEach(() => {
  resetCentreChatForTests();
  newSession = vi.fn();
  useSessionStore.setState({ newSession: newSession as never });
});

afterEach(() => {
  resetCentreChatForTests();
  useSessionStore.setState({ newSession: originalNewSession });
});

describe("empty centre", () => {
  it("opens a chat once both the layout and the socket are ready", () => {
    bindDockview(empty);
    noteLayoutSettled();
    expect(newSession).not.toHaveBeenCalled();
    noteTransportReady();
    expect(newSession).toHaveBeenCalledTimes(1);
  });

  it("opens a chat when the socket is ready first", () => {
    noteTransportReady();
    expect(newSession).not.toHaveBeenCalled();
    bindDockview(empty);
    noteLayoutSettled();
    expect(newSession).toHaveBeenCalledTimes(1);
  });

  it("opens a chat when only the edge rails are present", () => {
    noteTransportReady();
    bindDockview(railsOnly);
    noteLayoutSettled();
    expect(newSession).toHaveBeenCalledTimes(1);
  });

  it("does not open a second chat on a later hello", () => {
    bindDockview(empty);
    noteLayoutSettled();
    noteTransportReady();
    noteTransportReady();
    expect(newSession).toHaveBeenCalledTimes(1);
  });
});

describe("centre already has a panel", () => {
  it.each(["agent", "subagent", "editor"])(
    "leaves a persisted %s panel alone",
    (component) => {
      noteTransportReady();
      bindDockview(fakeApi([{ component, where: "grid" }]));
      noteLayoutSettled();
      expect(newSession).not.toHaveBeenCalled();
    },
  );
});

  it("does not spawn NEW when a restored center panel is still 0x0", () => {
    noteTransportReady();
    const host = {
      api: {
        id: "main",
        location: { type: "grid" },
        isVisible: true,
        width: 0,
        height: 0,
      },
      panels: [{ id: "agent-1", api: { component: "agent" } }],
    };
    bindDockview({
      panels: [
        {
          id: "agent-1",
          api: {
            component: "agent",
            location: { type: "grid" },
            group: host,
          },
        },
      ],
    } as unknown as DockviewApi);
    noteLayoutSettled();
    expect(newSession).not.toHaveBeenCalled();
  });


const flush = () => new Promise((r) => setTimeout(r, 0));

function track(): { done: boolean } {
  const state = { done: false };
  void whenCentreSettled().then(() => {
    state.done = true;
  });
  return state;
}

const agentInCentre = fakeApi([
  { component: "filetree", where: "left" },
  { component: "agent", where: "grid" },
]);
const editorOnlyCentre = fakeApi([{ component: "editor", where: "grid" }]);

describe("WP-170 whenCentreSettled — path A (empty centre → session/new)", () => {
  afterEach(() => {
    useSessionStore.setState({ pendingSessionOp: null, statusMessage: null });
  });

  it("A1 waits for the session/new reply (success)", async () => {
    newSession.mockImplementation(() =>
      useSessionStore.setState({ pendingSessionOp: "new_session" }),
    );
    const s = track();
    bindDockview(empty);
    noteLayoutSettled();
    noteTransportReady();
    await flush();
    expect(newSession).toHaveBeenCalledTimes(1);
    expect(s.done).toBe(false);
    useSessionStore.setState({ pendingSessionOp: null });
    await flush();
    expect(s.done).toBe(true);
  });

  it("A2 an error reply (pending op cleared by catch) also settles", async () => {
    newSession.mockImplementation(() =>
      useSessionStore.setState({ pendingSessionOp: "new_session" }),
    );
    const s = track();
    bindDockview(empty);
    noteLayoutSettled();
    noteTransportReady();
    useSessionStore.setState({
      pendingSessionOp: null,
      statusMessage: "Failed to create new session",
    });
    await flush();
    expect(s.done).toBe(true);
  });

  it("A3 settles at once when no session/new of ours went out", async () => {
    useSessionStore.setState({ pendingSessionOp: null });
    const s = track();
    bindDockview(empty);
    noteLayoutSettled();
    noteTransportReady();
    await flush();
    expect(s.done).toBe(true);
  });

  it("A4 only popout panels (main centre empty) takes path A", async () => {
    newSession.mockImplementation(() =>
      useSessionStore.setState({ pendingSessionOp: "new_session" }),
    );
    const s = track();
    bindDockview(fakeApi([{ component: "agent", where: "popout" }]));
    noteLayoutSettled();
    noteTransportReady();
    expect(newSession).toHaveBeenCalledTimes(1);
    useSessionStore.setState({ pendingSessionOp: null });
    await flush();
    expect(s.done).toBe(true);
  });
});

describe("WP-170 whenCentreSettled — path B (restored centre)", () => {
  afterEach(() => {
    useSessionStore.setState({ pendingSessionOp: null });
  });

  it("B1 waits for the session/list reply; no session/new", async () => {
    const s = track();
    bindDockview(agentInCentre);
    noteLayoutSettled();
    noteTransportReady();
    await flush();
    expect(newSession).not.toHaveBeenCalled();
    expect(s.done).toBe(false);
    noteSessionListSettled();
    await flush();
    expect(s.done).toBe(true);
  });

  it("B2 a panel attach settles as well", async () => {
    const s = track();
    bindDockview(agentInCentre);
    noteLayoutSettled();
    noteTransportReady();
    noteRootAttached();
    await flush();
    expect(s.done).toBe(true);
  });

  it("B3 a list reply that arrived before the layout settled still counts (no root panel)", async () => {
    const s = track();
    noteSessionListSettled();
    await flush();
    expect(s.done).toBe(false);
    bindDockview(editorOnlyCentre);
    noteLayoutSettled();
    noteTransportReady();
    await flush();
    expect(newSession).not.toHaveBeenCalled();
    expect(s.done).toBe(true);
  });
});

import { afterEach, describe, expect, it, vi } from "vitest";

import { setDockviewApi, useConnectionStore } from "../stores/connectionStore";
import {
  classifySession,
  clearPendingReveal,
  getPendingReveal,
  openKnownSessionPanel,
  openSessionPanel,
  openSubagentPanel,
  requestSeqReveal,
  subscribePendingReveal,
} from "./sessionPanelNav";

afterEach(() => {
  clearPendingReveal();
  setDockviewApi(null);
});

describe("sessionPanelNav", () => {
  it("keeps only the latest pending seq reveal", () => {
    const seen: number[] = [];
    const unsub = subscribePendingReveal(() => {
      seen.push(getPendingReveal()?.seq ?? -1);
    });
    requestSeqReveal("s1", 3);
    requestSeqReveal("s1", 9);
    expect(getPendingReveal()).toMatchObject({ sessionId: "s1", seq: 9 });
    expect(seen).toEqual([3, 9]);
    unsub();
  });

  it("activates an already-open panel without adding another", () => {
    const setActive = vi.fn();
    const addPanel = vi.fn();
    const ensureSubscribe = vi.fn(async () => {});
    useConnectionStore.setState({ ensureSubscribe } as never);
    setDockviewApi({
      getPanel: vi.fn(() => ({ api: { setActive } })),
      addPanel,
      groups: [],
      addGroup: vi.fn(),
    } as never);

    openSessionPanel("sess-open", 4);
    expect(setActive).toHaveBeenCalled();
    expect(addPanel).not.toHaveBeenCalled();
    expect(getPendingReveal()).toMatchObject({
      sessionId: "sess-open",
      seq: 4,
    });
  });

  it("opens a missing panel in the grid group", () => {
    const addPanel = vi.fn();
    setDockviewApi({
      getPanel: vi.fn(() => undefined),
      addPanel,
      addGroup: vi.fn(() => ({ id: "g-new" })),
      groups: [{ api: { location: { type: "grid" }, id: "g1" } }],
    } as never);
    openSessionPanel("sess-new");
    expect(addPanel).toHaveBeenCalledWith(
      expect.objectContaining({
        id: "agent-sess-new",
        component: "agent",
        params: { sessionId: "sess-new", sessionKind: "root" },
        position: { referenceGroup: "g1" },
      }),
    );
    expect(getPendingReveal()).toBeNull();
  });

  it("opens a missing read-only subagent panel in the grid group", () => {
    const addPanel = vi.fn();
    setDockviewApi({
      getPanel: vi.fn(() => undefined),
      addPanel,
      addGroup: vi.fn(() => ({ id: "g-new" })),
      groups: [{ api: { location: { type: "grid" }, id: "g1" } }],
    } as never);

    openSubagentPanel("child-new", 7);

    expect(addPanel).toHaveBeenCalledWith(
      expect.objectContaining({
        id: "subagent-child-new",
        component: "subagent",
        params: { sessionId: "child-new" },
        position: { referenceGroup: "g1" },
      }),
    );
    expect(getPendingReveal()).toMatchObject({
      sessionId: "child-new",
      seq: 7,
    });
  });

  it("focuses an already-open subagent panel without adding another", () => {
    const setActive = vi.fn();
    const addPanel = vi.fn();
    setDockviewApi({
      getPanel: (id: string) =>
        id === "subagent-child-x" ? { api: { setActive } } : undefined,
      addPanel,
      groups: [],
      addGroup: vi.fn(),
    } as never);

    openSubagentPanel("child-x");

    expect(setActive).toHaveBeenCalled();
    expect(addPanel).not.toHaveBeenCalled();
  });

  it("activates a legacy writable agent panel instead of adding a second host", () => {
    // `ensureSubscribe` is not refcounted: a restored `agent-<child>` panel may
    // already own the child. It must be the single host (AgentPanel fail-closes
    // a known child), never a second panel that double-subscribes.
    const setActive = vi.fn();
    const addPanel = vi.fn();
    setDockviewApi({
      getPanel: (id: string) =>
        id === "agent-child-y" ? { api: { setActive } } : undefined,
      addPanel,
      groups: [],
      addGroup: vi.fn(),
    } as never);

    openSubagentPanel("child-y");

    expect(setActive).toHaveBeenCalled();
    expect(addPanel).not.toHaveBeenCalled();
  });
});

describe("classifySession", () => {
  const sessions = [
    { id: "root-1", parent_session_id: null },
    { id: "child-1", parent_session_id: "root-1" },
  ];

  it("classifies a known session with no parent as root", () => {
    expect(classifySession(sessions, "root-1")).toBe("root");
  });

  it("classifies a known session with a parent as child", () => {
    expect(classifySession(sessions, "child-1")).toBe("child");
  });

  it("treats a session absent from the list as unknown (fail-closed)", () => {
    expect(classifySession(sessions, "ghost")).toBe("unknown");
    expect(classifySession([], "anything")).toBe("unknown");
  });
});

describe("openKnownSessionPanel", () => {
  const sessions = [
    { id: "root-1", parent_session_id: null },
    { id: "child-1", parent_session_id: "root-1" },
  ];

  function setApi() {
    const addPanel = vi.fn();
    setDockviewApi({
      getPanel: vi.fn(() => undefined),
      addPanel,
      addGroup: vi.fn(() => ({ id: "g-new" })),
      groups: [{ api: { location: { type: "grid" }, id: "g1" } }],
    } as never);
    return addPanel;
  }

  it("routes a confirmed root to the writable agent panel", () => {
    const addPanel = setApi();
    openKnownSessionPanel("root-1", sessions);
    expect(addPanel).toHaveBeenCalledWith(
      expect.objectContaining({
        id: "agent-root-1",
        component: "agent",
        params: { sessionId: "root-1", sessionKind: "root" },
      }),
    );
  });

  it("routes a known child to the read-only subagent panel", () => {
    const addPanel = setApi();
    openKnownSessionPanel("child-1", sessions);
    expect(addPanel).toHaveBeenCalledWith(
      expect.objectContaining({
        id: "subagent-child-1",
        component: "subagent",
      }),
    );
  });

  it("routes an unknown session to the read-only subagent panel", () => {
    const addPanel = setApi();
    openKnownSessionPanel("ghost", sessions, 5);
    expect(addPanel).toHaveBeenCalledWith(
      expect.objectContaining({ id: "subagent-ghost", component: "subagent" }),
    );
    expect(getPendingReveal()).toMatchObject({ sessionId: "ghost", seq: 5 });
  });
});

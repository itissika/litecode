import { afterEach, describe, expect, it, vi } from "vitest";

import { setDockviewApi, useConnectionStore } from "../stores/connectionStore";
import {
  clearPendingReveal,
  getPendingReveal,
  openSessionPanel,
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
    expect(getPendingReveal()).toMatchObject({ sessionId: "sess-open", seq: 4 });
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
        params: { sessionId: "sess-new" },
        position: { referenceGroup: "g1" },
      }),
    );
    expect(getPendingReveal()).toBeNull();
  });
});

import type { DockviewApi } from "dockview-react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { bindDockview } from "../dockview/workbench/host";
import { useSessionStore } from "../stores/sessionStore";
import {
  noteLayoutSettled,
  noteTransportReady,
  resetCentreChatForTests,
} from "./centreChat";

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

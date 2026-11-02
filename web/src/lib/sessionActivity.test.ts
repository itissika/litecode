import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { EMPTY_SLICE, useTurnStore } from "../stores/turnStore";
import { stubDesktopBridge } from "../test/desktopBridge";
import { startSessionActivityBridge } from "./sessionActivity";

const setSessionsActive = vi.fn(async () => {});

beforeEach(() => {
  setSessionsActive.mockClear();
  useTurnStore.setState({ byId: new Map() });
  window.litecode = stubDesktopBridge({ setSessionsActive });
});

afterEach(() => {
  delete window.litecode;
  useTurnStore.setState({ byId: new Map() });
});

describe("startSessionActivityBridge", () => {
  it("pushes the current state as soon as it starts", () => {
    useTurnStore.setState({
      byId: new Map([["s1", { ...EMPTY_SLICE, runState: "running" }]]),
    });

    const stop = startSessionActivityBridge();

    expect(setSessionsActive).toHaveBeenCalledWith(true);
    stop();
  });

  it("pushes again only when the busy flag actually flips", () => {
    const stop = startSessionActivityBridge();
    expect(setSessionsActive).toHaveBeenLastCalledWith(false);

    useTurnStore.setState({
      byId: new Map([["s1", { ...EMPTY_SLICE, runState: "running" }]]),
    });
    expect(setSessionsActive).toHaveBeenLastCalledWith(true);
    expect(setSessionsActive).toHaveBeenCalledTimes(2);

    // A second idle session is a new map but not a new answer.
    useTurnStore.setState({
      byId: new Map([
        ["s1", { ...EMPTY_SLICE, runState: "running" }],
        ["s2", { ...EMPTY_SLICE }],
      ]),
    });
    expect(setSessionsActive).toHaveBeenCalledTimes(2);

    useTurnStore.setState({ byId: new Map() });
    expect(setSessionsActive).toHaveBeenLastCalledWith(false);
    expect(setSessionsActive).toHaveBeenCalledTimes(3);
    stop();
  });

  it("stops pushing once detached", () => {
    startSessionActivityBridge()();
    useTurnStore.setState({
      byId: new Map([["s1", { ...EMPTY_SLICE, runState: "cancelling" }]]),
    });
    expect(setSessionsActive).toHaveBeenCalledTimes(1);
  });

  it("is a no-op in browser mode, where there is no host", () => {
    delete window.litecode;
    const stop = startSessionActivityBridge();
    expect(() => {
      useTurnStore.setState({
        byId: new Map([["s1", { ...EMPTY_SLICE, runState: "running" }]]),
      });
    }).not.toThrow();
    expect(setSessionsActive).not.toHaveBeenCalled();
    stop();
  });
});

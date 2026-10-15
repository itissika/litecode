import { act, render } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { RemoteDisconnectedEvent } from "../../../desktop/src/bridge-contract";
import { stubDesktopBridge } from "../test/desktopBridge";
import { StatusBar } from "./StatusBar";

afterEach(() => {
  Reflect.deleteProperty(window, "litecode");
});

describe("StatusBar remote-disconnected (A2)", () => {
  it("enters the disconnected style on the desktop event without changing text", () => {
    const bus: { emit?: (event: RemoteDisconnectedEvent) => void } = {};
    const unsubscribe = vi.fn();
    window.litecode = stubDesktopBridge({
      onRemoteDisconnected: (handler) => {
        bus.emit = handler;
        return unsubscribe;
      },
    });

    const { container, unmount } = render(<StatusBar sessionMode="remote" />);
    const root = container.querySelector(".status-bar-root");
    expect(root).not.toBeNull();
    expect(root?.hasAttribute("data-disconnected")).toBe(false);
    const textBefore = root?.textContent;

    expect(bus.emit).toBeDefined();
    act(() => bus.emit?.({ reason: "socket closed" }));

    expect(root?.hasAttribute("data-disconnected")).toBe(true);
    expect(root?.textContent).toBe(textBefore);

    unmount();
    expect(unsubscribe).toHaveBeenCalledTimes(1);
  });

  it("renders normally without a desktop bridge", () => {
    const { container } = render(<StatusBar />);
    const root = container.querySelector(".status-bar-root");
    expect(root?.hasAttribute("data-disconnected")).toBe(false);
  });
});

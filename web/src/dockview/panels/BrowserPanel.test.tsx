import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import type { IDockviewPanelProps } from "dockview-react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { BrowserPanel } from "./BrowserPanel";
import { stubDesktopBridge } from "../../test/desktopBridge";

const PANEL_ID = "browser-11111111-1111-4111-8111-111111111111";

class ResizeObserverStub {
  observe() {}
  unobserve() {}
  disconnect() {}
}

vi.stubGlobal("ResizeObserver", ResizeObserverStub);

type PanelState = {
  id: string;
  url: string;
  title: string;
  canGoBack: boolean;
  canGoForward: boolean;
  loading: boolean;
};

function state(overrides: Partial<PanelState> = {}): PanelState {
  return {
    id: PANEL_ID,
    url: "https://example.com/docs",
    title: "Docs",
    canGoBack: false,
    canGoForward: false,
    loading: false,
    ...overrides,
  };
}

/** The native page lives in the host; only the toolbar contract is under test. */
function panelProps(): IDockviewPanelProps<{ url?: string }> {
  return {
    params: {},
    api: {
      id: PANEL_ID,
      isVisible: true,
      isActive: true,
      title: "Browser",
      location: { type: "grid" },
      setTitle: vi.fn(),
      getParameters: () => ({ url: "" }),
      updateParameters: vi.fn(),
      onDidVisibilityChange: () => ({ dispose: () => {} }),
      onDidLocationChange: () => ({ dispose: () => {} }),
      onDidActiveChange: () => ({ dispose: () => {} }),
    },
  } as unknown as IDockviewPanelProps<{ url?: string }>;
}

afterEach(() => {
  cleanup();
  delete window.litecode;
});

describe("BrowserPanel page actions", () => {
  it("stops a pending load and reloads once it settles", async () => {
    const stop = vi.fn(async () => state({ loading: false }));
    const reload = vi.fn(async () => state({ loading: true }));
    window.litecode = stubDesktopBridge({
      browserCreate: async () => state({ loading: true }),
      browserStop: stop,
      browserReload: reload,
    });

    render(<BrowserPanel {...panelProps()} />);

    fireEvent.click(await screen.findByRole("button", { name: "Stop" }));
    expect(stop).toHaveBeenCalledWith(PANEL_ID);

    fireEvent.click(await screen.findByRole("button", { name: "Reload" }));
    expect(reload).toHaveBeenCalledWith(PANEL_ID);
    expect(await screen.findByRole("button", { name: "Stop" })).toBeTruthy();
  });

  it("reports the active page and disables the toolbar while the agent works", async () => {
    let activity: ((event: { id: string; busy: boolean }) => void) | undefined;
    const setActive = vi.fn();
    window.litecode = stubDesktopBridge({
      browserCreate: async () => state(),
      browserSetActive: setActive,
      onBrowserAgentActivity: (handler) => {
        activity = handler;
        return () => {
          activity = undefined;
        };
      },
    });

    render(<BrowserPanel {...panelProps()} />);

    expect(await screen.findByRole("button", { name: "Reload" })).toBeTruthy();
    expect(setActive).toHaveBeenCalledWith({ id: PANEL_ID, active: true });
    const address = screen.getByRole("textbox", { name: "Address" }) as HTMLInputElement;
    expect(address.disabled).toBe(false);

    act(() => activity?.({ id: PANEL_ID, busy: true }));
    expect(address.disabled).toBe(true);
    expect(screen.getByText("Agent is working on this page…")).toBeTruthy();

    act(() => activity?.({ id: PANEL_ID, busy: false }));
    expect(address.disabled).toBe(false);
  });

  it("keeps reload off until the page has an address", async () => {
    window.litecode = stubDesktopBridge({
      browserCreate: async () => state({ url: "about:blank" }),
      browserReload: vi.fn(),
    });

    render(<BrowserPanel {...panelProps()} />);

    const reload = (await screen.findByRole("button", {
      name: "Reload",
    })) as HTMLButtonElement;
    expect(reload.disabled).toBe(true);
    expect(screen.queryByRole("button", { name: "Stop" })).toBeNull();
  });
});

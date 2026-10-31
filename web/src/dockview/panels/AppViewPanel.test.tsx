import { act, cleanup, render, waitFor } from "@testing-library/react";
import type { IDockviewPanelProps } from "dockview-react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { AppView } from "../../api/apps";
import { useAppsStore } from "../../stores/appsStore";
import { useNativeViewOcclusionStore } from "../../stores/nativeViewOcclusionStore";
import { stubDesktopBridge } from "../../test/desktopBridge";
import { AppViewPanel } from "./AppViewPanel";

const APP: AppView = {
  id: "demo",
  name: "Demo",
  description: "",
  description_i18n: {},
  category: "general",
  icon: "",
  repo: null,
  docs_url: null,
  docs: null,
  source: "builtin",
  view_key: "demo",
  status: "running",
  running: true,
  url: "https://example.test/",
  builtin_view: null,
  port: null,
  can_start: true,
  can_open: true,
  trusted: true,
  definition_hash: null,
  trust_preview: null,
  permissions: [],
  single_instance: false,
  install: null,
  runtime_error: null,
} as unknown as AppView;

function fakePanel() {
  return {
    id: "app-panel",
    isActive: true,
    isVisible: true,
    location: { type: "grid" },
    onDidActiveChange: () => ({ dispose: () => {} }),
    onDidVisibilityChange: () => ({ dispose: () => {} }),
    close: vi.fn(),
    setTitle: vi.fn(),
  };
}

function props(api = fakePanel()): IDockviewPanelProps<{ appId: string }> {
  return { params: { appId: APP.id }, api } as unknown as IDockviewPanelProps<{
    appId: string;
  }>;
}

describe("AppViewPanel — native host contract", () => {
  beforeEach(() => {
    window.litecode = stubDesktopBridge({
      appViewCreate: vi.fn(async (request) => ({
        id: request.id,
        url: request.url,
        title: APP.name,
        canGoBack: false,
        canGoForward: false,
        loading: false,
      })),
      appViewDestroy: vi.fn(),
      appViewSetBounds: vi.fn(),
      appViewSetActive: vi.fn(),
      appViewSetVisible: vi.fn(),
      onAppViewState: () => () => {},
    });
    useAppsStore.setState({ apps: [APP], loading: false });
    window.ResizeObserver = class {
      observe() {}
      unobserve() {}
      disconnect() {}
    } as unknown as typeof ResizeObserver;
  });

  afterEach(() => {
    cleanup();
    useNativeViewOcclusionStore.getState().clearAll();
    useAppsStore.setState({ apps: [], loading: false });
    delete (window as Window & { litecode?: unknown }).litecode;
    delete (window as Window & { ResizeObserver?: unknown }).ResizeObserver;
  });

  it("exposes an app-kind native host with the same id its snapshot uses", async () => {
    render(<AppViewPanel {...props()} />);

    const host = await waitFor(() => {
      const node = document.querySelector<HTMLElement>("[data-native-view-host]");
      expect(node).toBeTruthy();
      return node!;
    });

    expect(host.dataset.nativeViewKind).toBe("app");
    const id = host.dataset.nativeViewId;
    expect(id).toMatch(/^app-/);

    act(() => {
      useNativeViewOcclusionStore.getState().setPlaceholder("app", id!, "");
    });

    const snapshot = document.querySelector(
      '[data-native-view-placeholder="app"]',
    );
    expect(snapshot).toBeTruthy();
  });
});
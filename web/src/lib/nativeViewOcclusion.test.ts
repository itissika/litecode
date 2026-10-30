import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { LitecodeDesktopBridge } from "../api/auth";
import {
  bindWindowRegistry,
  resetWindowsForTests,
} from "../dockview/workbench/windows";
import { useNativeViewOcclusionStore } from "../stores/nativeViewOcclusionStore";
import { stubDesktopBridge } from "../test/desktopBridge";
import {
  installNativeViewOcclusion,
  resetNativeViewOcclusionForTests,
} from "./nativeViewOcclusion";

const PANEL_ID = "browser-11111111-1111-4111-8111-111111111111";
const OTHER_ID = "browser-22222222-2222-4222-8222-222222222222";
const quietApi = {
  onDidAddPopoutGroup: () => ({ dispose() {} }),
  onDidRemovePopoutGroup: () => ({ dispose() {} }),
};

function setRect(element: HTMLElement, left: number, top: number, width: number, height: number) {
  vi.spyOn(element, "getBoundingClientRect").mockReturnValue({
    x: left,
    y: top,
    width,
    height,
    top,
    left,
    right: left + width,
    bottom: top + height,
    toJSON: () => ({}),
  } as DOMRect);
  (element as HTMLElement & { checkVisibility: () => boolean }).checkVisibility = () => true;
}

function addHost(id: string): HTMLElement {
  const host = document.createElement("div");
  host.dataset.nativeViewHost = "";
  host.dataset.nativeViewKind = "browser";
  host.dataset.nativeViewId = id;
  document.body.appendChild(host);
  setRect(host, 0, 0, 100, 100);
  return host;
}

function addOverlay(): HTMLElement {
  const overlay = document.createElement("div");
  overlay.dataset.litecodeOverlay = "dialog";
  document.body.appendChild(overlay);
  setRect(overlay, 50, 50, 100, 100);
  return overlay;
}

function bridgeWith(calls: string[]): LitecodeDesktopBridge {
  return stubDesktopBridge({
    nativeViewGetSnapshot: async ({ kind, id }) => {
      calls.push(`get:${kind}:${id}`);
      return "data:image/jpeg;base64,c25hcHNob3Q=";
    },
    nativeViewSetOccluded: ({ kind, id, occluded }) => {
      calls.push(`${occluded ? "hide" : "show"}:${kind}:${id}`);
    },
  });
}

beforeEach(() => {
  resetWindowsForTests();
  bindWindowRegistry(quietApi);
  window.requestAnimationFrame = ((callback: FrameRequestCallback) => {
    callback(0);
    return 1;
  }) as typeof window.requestAnimationFrame;
});

afterEach(() => {
  resetNativeViewOcclusionForTests();
  resetWindowsForTests();
  document.body.innerHTML = "";
  delete window.litecode;
  vi.restoreAllMocks();
});

describe("installNativeViewOcclusion", () => {
  it("hides the overlapping view and restores it when the overlay closes", async () => {
    const calls: string[] = [];
    window.litecode = bridgeWith(calls);
    addHost(PANEL_ID);
    const overlay = addOverlay();
    installNativeViewOcclusion();

    await vi.waitFor(() => {
      expect(calls).toEqual([
        `get:browser:${PANEL_ID}`,
        `hide:browser:${PANEL_ID}`,
      ]);
    });
    expect(
      useNativeViewOcclusionStore.getState().placeholders[`browser:${PANEL_ID}`]?.dataUrl,
    ).toContain("data:image/jpeg");

    overlay.remove();
    await vi.waitFor(() => {
      expect(calls.at(-1)).toBe(`show:browser:${PANEL_ID}`);
    });
    expect(
      useNativeViewOcclusionStore.getState().placeholders[`browser:${PANEL_ID}`],
    ).toBeUndefined();
  });

  it("leaves a non-overlapping view alone", async () => {
    const calls: string[] = [];
    window.litecode = bridgeWith(calls);
    addHost(PANEL_ID);
    const other = addHost(OTHER_ID);
    setRect(other, 200, 0, 100, 100);
    addOverlay();
    installNativeViewOcclusion();

    await vi.waitFor(() => {
      expect(calls).toEqual([
        `get:browser:${PANEL_ID}`,
        `hide:browser:${PANEL_ID}`,
      ]);
    });
    expect(calls.some((call) => call.includes(OTHER_ID))).toBe(false);
  });

  it("cancels a snapshot that resolves after the overlay closes", async () => {
    const calls: string[] = [];
    let resolveSnapshot: ((value: string) => void) | null = null;
    window.litecode = stubDesktopBridge({
      nativeViewGetSnapshot: ({ kind, id }) => {
        calls.push(`get:${kind}:${id}`);
        return new Promise((resolve) => {
          resolveSnapshot = resolve;
        });
      },
      nativeViewSetOccluded: ({ kind, id, occluded }) => {
        calls.push(`${occluded ? "hide" : "show"}:${kind}:${id}`);
      },
    });
    addHost(PANEL_ID);
    const overlay = addOverlay();
    installNativeViewOcclusion();

    await vi.waitFor(() => {
      expect(calls).toContain(`get:browser:${PANEL_ID}`);
    });
    overlay.remove();
    await vi.waitFor(() => {
      expect(calls).not.toContain(`hide:browser:${PANEL_ID}`);
    });
    const resolve = resolveSnapshot as ((value: string) => void) | null;
    resolve?.("data:image/jpeg;base64,c25hcHNob3Q=");
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(calls).not.toContain(`hide:browser:${PANEL_ID}`);
  });

  it("releases every view when the manager stops", async () => {
    const calls: string[] = [];
    window.litecode = bridgeWith(calls);
    addHost(PANEL_ID);
    addOverlay();
    const stop = installNativeViewOcclusion();

    await vi.waitFor(() => {
      expect(calls).toContain(`hide:browser:${PANEL_ID}`);
    });
    stop();
    expect(calls.at(-1)).toBe(`show:browser:${PANEL_ID}`);
  });
});

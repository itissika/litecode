import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { LitecodeDesktopBridge } from "../api/auth";
import type { GroupLike } from "../dockview/workbench/readGroup";
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
type PopoutLike = {
  id: string;
  window: Window;
  group: GroupLike;
};

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

/** Flip the visibility the manager reads (jsdom has no checkVisibility). */
function setVisible(element: HTMLElement, visible: boolean): void {
  (element as HTMLElement & { checkVisibility: () => boolean }).checkVisibility = () =>
    visible;
}

function addHost(id: string, doc: Document = document): HTMLElement {
  const host = doc.createElement("div");
  host.dataset.nativeViewHost = "";
  host.dataset.nativeViewKind = "browser";
  host.dataset.nativeViewId = id;
  doc.body.appendChild(host);
  setRect(host, 0, 0, 100, 100);
  return host;
}

function addOverlay(
  kind = "dialog",
  doc: Document = document,
): HTMLElement {
  const overlay = doc.createElement("div");
  overlay.dataset.litecodeOverlay = kind;
  doc.body.appendChild(overlay);
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

  it("releases a view whose host is no longer visible, then hides it again only when it is", async () => {
    const calls: string[] = [];
    window.litecode = bridgeWith(calls);
    const host = addHost(PANEL_ID);
    const overlay = addOverlay();
    installNativeViewOcclusion();

    await vi.waitFor(() => {
      expect(calls).toEqual([
        `get:browser:${PANEL_ID}`,
        `hide:browser:${PANEL_ID}`,
      ]);
    });
    expect(
      useNativeViewOcclusionStore.getState().placeholders[`browser:${PANEL_ID}`],
    ).toBeTruthy();

    // The panel went into a hidden tab: a view that cannot be seen must not
    // stay hidden behind a still frame.
    setVisible(host, false);
    window.dispatchEvent(new Event("resize"));

    await vi.waitFor(() => {
      expect(calls.at(-1)).toBe(`show:browser:${PANEL_ID}`);
    });
    expect(
      useNativeViewOcclusionStore.getState().placeholders[`browser:${PANEL_ID}`],
    ).toBeUndefined();

    // Coming back with nothing over it must not re-hide: the state was
    // released, not merely paused.
    setVisible(host, true);
    overlay.remove();
    window.dispatchEvent(new Event("resize"));
    await new Promise((resolve) => setTimeout(resolve, 0));

    expect(calls.filter((call) => call.startsWith("hide:"))).toHaveLength(1);

    // …and the same view is still managed after that round trip.
    addOverlay();
    window.dispatchEvent(new Event("resize"));

    await vi.waitFor(() => {
      expect(calls.filter((call) => call.startsWith("hide:"))).toHaveLength(2);
    });
  });

  it("releases a view whose host is removed, without waiting for a resize", async () => {
    const calls: string[] = [];
    window.litecode = bridgeWith(calls);
    const host = addHost(PANEL_ID);
    addOverlay();
    installNativeViewOcclusion();

    await vi.waitFor(() => {
      expect(calls).toContain(`hide:browser:${PANEL_ID}`);
    });

    host.remove();

    await vi.waitFor(() => {
      expect(calls.at(-1)).toBe(`show:browser:${PANEL_ID}`);
    });
    expect(
      useNativeViewOcclusionStore.getState().placeholders[`browser:${PANEL_ID}`],
    ).toBeUndefined();
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

  it("hides an instant overlay before its snapshot resolves", async () => {
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
    addOverlay("context-menu");
    installNativeViewOcclusion();

    await vi.waitFor(() => {
      expect(calls).toEqual([
        `hide:browser:${PANEL_ID}`,
        `get:browser:${PANEL_ID}`,
      ]);
    });
    expect(
      useNativeViewOcclusionStore.getState().placeholders[`browser:${PANEL_ID}`]
        ?.dataUrl,
    ).toBe("");

    const resolve = resolveSnapshot as ((value: string) => void) | null;
    resolve?.("data:image/jpeg;base64,c25hcHNob3Q=");
    await vi.waitFor(() => {
      expect(
        useNativeViewOcclusionStore.getState().placeholders[`browser:${PANEL_ID}`]
          ?.dataUrl,
      ).toContain("data:image/jpeg");
    });
  });

  it("hides every overlapping view before any instant snapshot resolves", async () => {
    const calls: string[] = [];
    window.litecode = stubDesktopBridge({
      nativeViewGetSnapshot: ({ kind, id }) => {
        calls.push(`get:${kind}:${id}`);
        return new Promise(() => undefined);
      },
      nativeViewSetOccluded: ({ kind, id, occluded }) => {
        calls.push(`${occluded ? "hide" : "show"}:${kind}:${id}`);
      },
    });
    addHost(PANEL_ID);
    addHost(OTHER_ID);
    addOverlay("context-menu");
    installNativeViewOcclusion();

    await vi.waitFor(() => {
      expect(calls.filter((call) => call.startsWith("hide:"))).toEqual([
        `hide:browser:${PANEL_ID}`,
        `hide:browser:${OTHER_ID}`,
      ]);
    });
  });

  it.each(["dv-context-menu", "monaco-menu"])(
    "recognizes %s as an instant blocker",
    async (className) => {
      const calls: string[] = [];
      window.litecode = bridgeWith(calls);
      addHost(PANEL_ID);
      const menu = document.createElement("div");
      menu.className = className;
      document.body.appendChild(menu);
      setRect(menu, 50, 50, 100, 100);
      installNativeViewOcclusion();

      await vi.waitFor(() => {
        expect(calls).toEqual([
          `hide:browser:${PANEL_ID}`,
          `get:browser:${PANEL_ID}`,
        ]);
      });
    },
  );

  it("does not hide for a contextmenu without an overlay", async () => {
    const calls: string[] = [];
    window.litecode = bridgeWith(calls);
    addHost(PANEL_ID);
    installNativeViewOcclusion();

    document.body.dispatchEvent(
      new MouseEvent("contextmenu", { bubbles: true, cancelable: true }),
    );
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(calls).toEqual([]);
  });

  it("turns an actual tab drag into an instant tab-drag overlay", async () => {
    const calls: string[] = [];
    window.litecode = bridgeWith(calls);
    addHost(PANEL_ID);
    const tab = document.createElement("div");
    tab.className = "dv-tab";
    document.body.appendChild(tab);
    installNativeViewOcclusion();

    tab.dispatchEvent(
      new MouseEvent("pointerdown", { bubbles: true, clientX: 1, clientY: 1 }),
    );
    tab.dispatchEvent(
      new MouseEvent("pointermove", { bubbles: true, clientX: 9, clientY: 1 }),
    );

    await vi.waitFor(() => {
      expect(
        document.querySelector('[data-litecode-overlay="tab-drag"]'),
      ).not.toBeNull();
    });
    const overlay = document.querySelector<HTMLElement>(
      '[data-litecode-overlay="tab-drag"]',
    );
    if (!overlay) throw new Error("tab-drag overlay missing");
    setRect(overlay, 0, 0, 100, 100);
    window.dispatchEvent(new Event("resize"));

    await vi.waitFor(() => {
      expect(calls).toContain(`hide:browser:${PANEL_ID}`);
    });

    window.dispatchEvent(new MouseEvent("pointerup", { bubbles: true }));
    await vi.waitFor(() => {
      expect(
        document.querySelector('[data-litecode-overlay="tab-drag"]'),
      ).toBeNull();
    });
    await vi.waitFor(() => {
      expect(calls.at(-1)).toBe(`show:browser:${PANEL_ID}`);
    });
  });

  it("drives popout hosts through the main renderer bridge", async () => {
    const calls: string[] = [];
    window.litecode = bridgeWith(calls);
    const frame = document.createElement("iframe");
    document.body.appendChild(frame);
    const popout = frame.contentWindow;
    if (!popout) throw new Error("iframe window unavailable");

    addHost(PANEL_ID, popout.document);
    const overlay = addOverlay("context-menu", popout.document);

    const popouts: { add: ((popout: PopoutLike) => void) | null } = {
      add: null,
    };
    bindWindowRegistry({
      onDidAddPopoutGroup: (cb) => {
        popouts.add = cb;
        return { dispose() {} };
      },
      onDidRemovePopoutGroup: () => ({ dispose() {} }),
    });
    installNativeViewOcclusion();
    popouts.add?.({
      id: "dock-1",
      window: popout,
      group: {
        id: "dock-1",
        api: {
          id: "dock-1",
          location: {
            type: "popout",
            popoutUrl:
              "/popout.html?dock=11111111-1111-4111-8111-111111111111",
          },
        },
      },
    });

    await vi.waitFor(() => {
      expect(calls).toContain(`hide:browser:${PANEL_ID}`);
    });
    overlay.remove();
    popout.dispatchEvent(new Event("resize"));
    await vi.waitFor(() => {
      expect(calls.at(-1)).toBe(`show:browser:${PANEL_ID}`);
    });
  });
});

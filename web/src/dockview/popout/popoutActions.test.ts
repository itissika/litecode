import { afterEach, describe, expect, it, vi } from "vitest";

import { bindDockview } from "../workbench/host";
import { bindWindowRegistry, resetWindowsForTests } from "../workbench/windows";
import {
  POPOUT_ACTION_CLASS,
  POPOUT_ACTIONS_CLASS,
  bindPopoutActions,
  type PopoutActionsHost,
} from "./popoutActions";

const DOCK = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee";

function popoutWindow(body: string, href = `http://127.0.0.1/popout.html?dock=${DOCK}`): Window {
  const doc = document.implementation.createHTMLDocument("popout");
  doc.body.innerHTML = body;
  const target = new EventTarget();
  return {
    document: doc,
    location: { href },
    closed: false,
    addEventListener: target.addEventListener.bind(target),
    removeEventListener: target.removeEventListener.bind(target),
    requestAnimationFrame: (cb: FrameRequestCallback) => {
      cb(0);
      return 1;
    },
    cancelAnimationFrame: () => {},
  } as unknown as Window;
}

const HORIZONTAL = `
  <div class="dv-tabs-and-actions-container">
    <div class="dv-void-container" id="void-a"></div>
  </div>
  <div class="dv-tabs-and-actions-container">
    <div class="dv-void-container" id="void-b"></div>
  </div>
  <div class="dv-tabs-and-actions-container dv-groupview-header-vertical">
    <div class="dv-void-container" id="void-vertical"></div>
  </div>
`;

function host(overrides: Partial<PopoutActionsHost> = {}): PopoutActionsHost {
  return {
    setAlwaysOnTop: async (_dockId, onTop) => onTop,
    windowControl: {
      minimize: async () => {},
      toggleMaximize: async () => true,
      isMaximized: async () => false,
    },
    ...overrides,
  };
}

function button(win: Window, action: string): HTMLButtonElement {
  const found = win.document.querySelector<HTMLButtonElement>(
    `.dv-void-container > .${POPOUT_ACTIONS_CLASS} [data-lc-action="${action}"]`,
  );
  if (!found) throw new Error(`missing ${action} button`);
  return found;
}

describe("bindPopoutActions", () => {
  const stops: Array<() => void> = [];

  afterEach(() => {
    for (const stop of stops) stop();
    stops.length = 0;
    bindDockview(null);
    resetWindowsForTests();
  });

  it("puts one control row in each horizontal void", () => {
    const win = popoutWindow(HORIZONTAL);
    stops.push(bindPopoutActions(win, host()));
    expect(win.document.querySelectorAll(`.${POPOUT_ACTIONS_CLASS}`)).toHaveLength(2);
    expect(
      win.document.getElementById("void-vertical")?.querySelector(`.${POPOUT_ACTIONS_CLASS}`),
    ).toBeNull();
    const actions = [...win.document.querySelectorAll(`.${POPOUT_ACTION_CLASS}`)].map((el) =>
      el.getAttribute("data-lc-action"),
    );
    expect(actions).toEqual(["pin", "minimize", "maximize", "close-all", "pin", "minimize", "maximize", "close-all"]);
  });

  it("leaves out the OS window buttons when the host cannot control one", () => {
    const win = popoutWindow(HORIZONTAL);
    stops.push(bindPopoutActions(win, host({ windowControl: undefined })));
    const actions = [...win.document.querySelectorAll(`.${POPOUT_ACTION_CLASS}`)].map((el) =>
      el.getAttribute("data-lc-action"),
    );
    expect(actions).toEqual(["pin", "close-all", "pin", "close-all"]);
  });

  it("pins only that popout and keeps every row in step", async () => {
    const calls: Array<{ dockId: string; onTop: boolean }> = [];
    const win = popoutWindow(HORIZONTAL);
    stops.push(
      bindPopoutActions(
        win,
        host({
          setAlwaysOnTop: async (dockId, onTop) => {
            calls.push({ dockId, onTop });
            return onTop;
          },
        }),
      ),
    );

    button(win, "pin").dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
    await vi.waitFor(() => {
      expect(calls).toEqual([{ dockId: DOCK, onTop: true }]);
    });
    expect(
      [...win.document.querySelectorAll(`[data-lc-action="pin"]`)].every(
        (el) => el.getAttribute("aria-pressed") === "true",
      ),
    ).toBe(true);
  });

  it("minimizes this window through the host", async () => {
    const minimize = vi.fn(async () => {});
    const win = popoutWindow(HORIZONTAL);
    stops.push(bindPopoutActions(win, host({ windowControl: { minimize, toggleMaximize: async () => false, isMaximized: async () => false } })));
    button(win, "minimize").dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
    await vi.waitFor(() => expect(minimize).toHaveBeenCalledWith(DOCK));
  });

  it("swaps maximize for restore after a toggle", async () => {
    const toggleMaximize = vi.fn(async () => true);
    const win = popoutWindow(HORIZONTAL);
    stops.push(
      bindPopoutActions(
        win,
        host({ windowControl: { minimize: async () => {}, toggleMaximize, isMaximized: async () => false } }),
      ),
    );
    expect(button(win, "maximize").getAttribute("aria-label")).toBe("Maximize");
    button(win, "maximize").dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
    await vi.waitFor(() => expect(button(win, "maximize").getAttribute("aria-label")).toBe("Restore"));
    expect(toggleMaximize).toHaveBeenCalledWith(DOCK);
  });

  it("closes every tab of the window it sits in", () => {
    const closed: string[] = [];
    const group = {
      api: { id: "group-1", location: { type: "popout", popoutUrl: `/popout.html?dock=${DOCK}` } },
      panels: [
        { id: "src/a.ts", api: { component: "editor" } },
        { id: "src/b.ts", api: { component: "editor" } },
      ],
    };
    bindWindowRegistry({
      onDidAddPopoutGroup(cb: (popout: { id: string; window: Window; group: unknown }) => void) {
        cb({ id: "group-1", window: popoutWindow(HORIZONTAL), group });
        return { dispose() {} };
      },
      onDidRemovePopoutGroup() {
        return { dispose() {} };
      },
    } as never);
    bindDockview({
      getPanel: (id: string) => ({ api: { close: () => closed.push(id) } }),
    } as never);

    const win = popoutWindow(HORIZONTAL);
    stops.push(bindPopoutActions(win, host()));
    button(win, "close-all").dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
    expect(closed).toEqual(["src/a.ts", "src/b.ts"]);
  });

  it("blocks the press that would start a tab-bar drag", () => {
    const win = popoutWindow(HORIZONTAL);
    stops.push(bindPopoutActions(win, host()));
    const event = new MouseEvent("mousedown", { bubbles: true, cancelable: true });
    button(win, "pin").dispatchEvent(event);
    expect(event.defaultPrevented).toBe(true);
  });

  it("leaves a window without a dock id alone", () => {
    const win = popoutWindow(HORIZONTAL, "http://127.0.0.1/popout.html");
    const stop = bindPopoutActions(win, host());
    expect(win.document.querySelector(`.${POPOUT_ACTIONS_CLASS}`)).toBeNull();
    stop();
  });

  it("removes the rows when the popout watch stops", () => {
    const win = popoutWindow(HORIZONTAL);
    const stop = bindPopoutActions(win, host());
    expect(win.document.querySelectorAll(`.${POPOUT_ACTIONS_CLASS}`)).toHaveLength(2);
    stop();
    expect(win.document.querySelector(`.${POPOUT_ACTIONS_CLASS}`)).toBeNull();
  });
});

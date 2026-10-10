import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { installBrowserChromeObscure, pushBrowserObscure } from "./browserObscure";
import { bindWindowRegistry, resetWindowsForTests } from "../dockview/workbench/windows";

const quietApi = {
  onDidAddPopoutGroup: () => ({ dispose() {} }),
  onDidRemovePopoutGroup: () => ({ dispose() {} }),
};

beforeEach(() => {
  resetWindowsForTests();
  bindWindowRegistry(quietApi);
});

afterEach(() => {
  resetWindowsForTests();
  delete window.litecode;
  vi.useRealTimers();
});

describe("pushBrowserObscure", () => {
  it("stays hidden until every lease is released", () => {
    const calls: boolean[] = [];
    window.litecode = {
      browserSetObscured: (obscured) => calls.push(obscured),
    };
    const first = pushBrowserObscure();
    const second = pushBrowserObscure();
    expect(calls.at(-1)).toBe(true);
    first();
    expect(calls.at(-1)).toBe(true);
    second();
    second();
    expect(calls.at(-1)).toBe(false);
  });
});

describe("installBrowserChromeObscure", () => {
  it("hides pages while a context menu is open", () => {
    vi.useFakeTimers();
    const calls: boolean[] = [];
    window.litecode = {
      browserSetObscured: (obscured) => calls.push(obscured),
    };
    const stop = installBrowserChromeObscure();
    window.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true }));
    expect(calls.at(-1)).toBe(true);
    vi.runAllTimers();
    window.dispatchEvent(new PointerEvent("pointerup", { bubbles: true }));
    expect(calls.at(-1)).toBe(false);
    stop();
  });

  it("hides pages once a tab drag actually moves", () => {
    const calls: boolean[] = [];
    window.litecode = {
      browserSetObscured: (obscured) => calls.push(obscured),
    };
    const stop = installBrowserChromeObscure();
    const tab = document.createElement("div");
    tab.className = "dv-tab";
    document.body.appendChild(tab);
    tab.dispatchEvent(
      new PointerEvent("pointerdown", { bubbles: true, clientX: 0, clientY: 0, button: 0 }),
    );
    expect(calls.at(-1)).toBeUndefined();
    window.dispatchEvent(
      new PointerEvent("pointermove", { bubbles: true, clientX: 8, clientY: 0, button: 0 }),
    );
    expect(calls.at(-1)).toBe(true);
    window.dispatchEvent(new PointerEvent("pointerup", { bubbles: true }));
    expect(calls.at(-1)).toBe(false);
    tab.remove();
    stop();
  });

  it("hides pages for a popout context menu and ignores it after unregister", () => {
    vi.useFakeTimers();
    const calls: boolean[] = [];
    window.litecode = {
      browserSetObscured: (obscured) => calls.push(obscured),
    };
    const target = new EventTarget();
    const child = {
      closed: false,
      document: document.implementation.createHTMLDocument("popout"),
      location: { href: "/popout.html?dock=cccccccc-dddd-4eee-8fff-000000000001" },
      addEventListener: target.addEventListener.bind(target),
      removeEventListener: target.removeEventListener.bind(target),
      dispatchEvent: (event: Event) => target.dispatchEvent(event),
    } as unknown as Window;
    let onRemove: (popout: { id: string; window: Window; group: { api: { id: string } } }) => void =
      () => {};
    resetWindowsForTests();
    const stopRegistry = bindWindowRegistry({
      onDidAddPopoutGroup: (cb) => {
        cb({
          id: "group-1",
          window: child,
          group: {
            api: {
              id: "group-1",
              location: {
                type: "popout",
                popoutUrl: "/popout.html?dock=cccccccc-dddd-4eee-8fff-000000000001",
              },
            },
          },
        });
        return { dispose() {} };
      },
      onDidRemovePopoutGroup: (cb) => {
        onRemove = cb;
        return { dispose() {} };
      },
    });
    const stop = installBrowserChromeObscure();
    child.dispatchEvent(new MouseEvent("contextmenu"));
    expect(calls.at(-1)).toBe(true);
    vi.runAllTimers();
    child.dispatchEvent(new PointerEvent("pointerup"));
    expect(calls.at(-1)).toBe(false);
    const before = calls.length;
    onRemove({ id: "group-1", window: child, group: { api: { id: "group-1" } } });
    child.dispatchEvent(new MouseEvent("contextmenu"));
    expect(calls.length).toBe(before);
    stop();
    stopRegistry();
  });
});

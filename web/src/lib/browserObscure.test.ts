import { afterEach, describe, expect, it, vi } from "vitest";

import { installBrowserChromeObscure, pushBrowserObscure } from "./browserObscure";

afterEach(() => {
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
});

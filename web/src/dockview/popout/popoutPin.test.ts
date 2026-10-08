import { afterEach, describe, expect, it, vi } from "vitest";

import { POPOUT_PIN_CLASS, bindPopoutPin } from "./popoutPin";

const DOCK = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee";

function popoutWindow(body: string, href = `http://127.0.0.1/popout.html?dock=${DOCK}`): Window {
  const doc = document.implementation.createHTMLDocument("popout");
  doc.body.innerHTML = body;
  return { document: doc, location: { href }, closed: false } as unknown as Window;
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

describe("bindPopoutPin", () => {
  const stops: Array<() => void> = [];

  afterEach(() => {
    for (const stop of stops) stop();
    stops.length = 0;
  });

  it("pins only that popout and keeps every horizontal void in step", async () => {
    const calls: Array<{ dockId: string; onTop: boolean }> = [];
    const win = popoutWindow(HORIZONTAL);
    stops.push(
      bindPopoutPin(win, async (dockId, onTop) => {
        calls.push({ dockId, onTop });
        return onTop;
      }),
    );

    const buttons = () => [...win.document.querySelectorAll(`.${POPOUT_PIN_CLASS}`)];
    expect(buttons()).toHaveLength(2);
    expect(win.document.getElementById("void-vertical")?.querySelector(`.${POPOUT_PIN_CLASS}`)).toBeNull();
    expect(buttons().every((button) => button.getAttribute("aria-pressed") === "false")).toBe(true);

    buttons()[0]?.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
    await vi.waitFor(() => {
      expect(calls).toEqual([{ dockId: DOCK, onTop: true }]);
      expect(buttons().every((button) => button.getAttribute("aria-pressed") === "true")).toBe(true);
    });
    expect(buttons().every((button) => button.getAttribute("aria-label") === "Unpin")).toBe(true);
    await new Promise((resolve) => setTimeout(resolve, 0));

    buttons()[1]?.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
    await vi.waitFor(() => {
      expect(calls).toEqual([
        { dockId: DOCK, onTop: true },
        { dockId: DOCK, onTop: false },
      ]);
    });
    expect(buttons().every((button) => button.getAttribute("aria-pressed") === "false")).toBe(true);
  });

  it("blocks the press that would start a tab-bar drag", () => {
    const win = popoutWindow(HORIZONTAL);
    stops.push(bindPopoutPin(win, async () => true));
    const button = win.document.querySelector(`.${POPOUT_PIN_CLASS}`);
    if (!button) throw new Error("expected a pin button");
    const event = new MouseEvent("mousedown", { bubbles: true, cancelable: true });
    button.dispatchEvent(event);
    expect(event.defaultPrevented).toBe(true);
  });

  it("gives a void that appears later the current pin state", async () => {
    const win = popoutWindow(
      `<div class="dv-tabs-and-actions-container"><div class="dv-void-container" id="void-a"></div></div>`,
    );
    stops.push(bindPopoutPin(win, async () => true));
    win.document.querySelector(`.${POPOUT_PIN_CLASS}`)?.dispatchEvent(
      new MouseEvent("click", { bubbles: true, cancelable: true }),
    );
    await vi.waitFor(() => {
      expect(win.document.querySelector(`.${POPOUT_PIN_CLASS}`)?.getAttribute("aria-pressed")).toBe(
        "true",
      );
    });

    const row = win.document.createElement("div");
    row.className = "dv-tabs-and-actions-container";
    const voidEl = win.document.createElement("div");
    voidEl.className = "dv-void-container";
    row.appendChild(voidEl);
    win.document.body.appendChild(row);

    await vi.waitFor(() => {
      expect(voidEl.querySelector(`.${POPOUT_PIN_CLASS}`)?.getAttribute("aria-pressed")).toBe("true");
    });
  });

  it("leaves a window without a dock id alone", () => {
    const win = popoutWindow(HORIZONTAL, "http://127.0.0.1/popout.html");
    const stop = bindPopoutPin(win, async () => true);
    expect(win.document.querySelector(`.${POPOUT_PIN_CLASS}`)).toBeNull();
    stop();
  });

  it("removes the buttons when the popout watch stops", () => {
    const win = popoutWindow(HORIZONTAL);
    const stop = bindPopoutPin(win, async () => true);
    expect(win.document.querySelectorAll(`.${POPOUT_PIN_CLASS}`)).toHaveLength(2);
    stop();
    expect(win.document.querySelector(`.${POPOUT_PIN_CLASS}`)).toBeNull();
  });
});

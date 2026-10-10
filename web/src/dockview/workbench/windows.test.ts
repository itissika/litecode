import { afterEach, describe, expect, it, vi } from "vitest";

import { joinPopoutWindow } from "./commands";
import { bindDockview } from "./host";
import { popoutWindows } from "./queries";
import {
  bindWindowRegistry,
  getWindow,
  getWindows,
  onDidRegisterWindow,
  resetWindowsForTests,
  subscribeWindows,
} from "./windows";

const DOCK = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee";

interface PopoutRef {
  id: string;
  window: Window;
  group: {
    api: {
      id?: string;
      location?: { type?: string; popoutUrl?: string };
    };
  };
}

function popoutEvents() {
  let onAdd: (popout: PopoutRef) => void = () => {};
  let onRemove: (popout: PopoutRef) => void = () => {};
  return {
    api: {
      onDidAddPopoutGroup(cb: (popout: PopoutRef) => void) {
        onAdd = cb;
        return { dispose() {} };
      },
      onDidRemovePopoutGroup(cb: (popout: PopoutRef) => void) {
        onRemove = cb;
        return { dispose() {} };
      },
    },
    add(popout: PopoutRef) {
      onAdd(popout);
    },
    remove(popout: PopoutRef) {
      onRemove(popout);
    },
  };
}

function eventWindow(): Window {
  const target = new EventTarget();
  const doc = document.implementation.createHTMLDocument("popout");
  return {
    closed: false,
    document: doc,
    location: { href: "" },
    addEventListener: target.addEventListener.bind(target),
    removeEventListener: target.removeEventListener.bind(target),
    dispatchEvent: (event: Event) => target.dispatchEvent(event),
  } as unknown as Window;
}

function popout(win: Window, popoutUrl?: string): PopoutRef {
  return {
    id: "group-1",
    window: win,
    group: {
      api: {
        id: "group-1",
        location: { type: "popout", popoutUrl },
      },
    },
  };
}

afterEach(() => {
  resetWindowsForTests();
  bindDockview(null);
});

describe("window registry", () => {
  it("registers the main window once", () => {
    const events = popoutEvents();
    bindWindowRegistry(events.api);
    bindWindowRegistry(events.api);
    expect(getWindows().filter((entry) => entry.dockId === null)).toHaveLength(1);
  });

  it("registers a popout once and drops it when the group closes", () => {
    const events = popoutEvents();
    const seen: string[] = [];
    const off = onDidRegisterWindow((entry) => {
      if (entry.dockId) seen.push(entry.dockId);
    });
    bindWindowRegistry(events.api);
    const win = eventWindow();
    const item = popout(win, `/popout.html?dock=${DOCK}`);
    events.add(item);
    events.add(item);
    expect(seen).toEqual([DOCK]);
    expect(popoutWindows()).toEqual([{ dockId: DOCK, groupId: "group-1" }]);
    events.remove(item);
    expect(popoutWindows()).toEqual([]);
    off();
  });

  it("waits for a dock id that arrives after the group", () => {
    const events = popoutEvents();
    bindWindowRegistry(events.api);
    const win = eventWindow();
    const item = popout(win);
    events.add(item);
    expect(popoutWindows()).toEqual([]);
    item.group.api.location = { type: "popout", popoutUrl: `/popout.html?dock=${DOCK}` };
    win.dispatchEvent(new Event("load"));
    win.dispatchEvent(new Event("load"));
    expect(popoutWindows()).toEqual([{ dockId: DOCK, groupId: "group-1" }]);
  });

  it("attaches a subscriber to each window and detaches it on unregister", () => {
    const events = popoutEvents();
    bindWindowRegistry(events.api);
    let attached = 0;
    const stop = subscribeWindows(() => {
      attached += 1;
      return () => {
        attached -= 1;
      };
    });
    expect(attached).toBe(1);
    const item = popout(eventWindow(), `/popout.html?dock=${DOCK}`);
    events.add(item);
    expect(attached).toBe(2);
    events.remove(item);
    expect(attached).toBe(1);
    stop();
    expect(attached).toBe(0);
  });

  it("resolves a window from an event view and falls back to the main window", () => {
    const events = popoutEvents();
    bindWindowRegistry(events.api);
    const child = eventWindow();
    expect(getWindow({ view: child } as unknown as Event)).toBe(child);
    expect(getWindow(null)).toBe(window);
    expect(getWindow(document.body)).toBe(window);
  });

  it("joins a panel only by dock id", () => {
    const moveTo = vi.fn();
    const events = popoutEvents();
    const win = eventWindow();
    const item = popout(win, `/popout.html?dock=${DOCK}`);
    bindDockview({
      getPanel: () => ({ api: { moveTo } }),
    } as never);
    bindWindowRegistry(events.api);
    events.add(item);
    joinPopoutWindow("src/a.ts", "group-1");
    expect(moveTo).not.toHaveBeenCalled();
    joinPopoutWindow("src/a.ts", DOCK);
    expect(moveTo).toHaveBeenCalledWith({ group: item.group });
  });
});

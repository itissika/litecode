import { afterEach, describe, expect, it } from "vitest";

import {
  bindWindowActivity,
  lastUserDockId,
  noteUserWindow,
  resetActivityForTests,
} from "./activity";
import type { GroupLike } from "./readGroup";
import { bindWindowRegistry, resetWindowsForTests } from "./windows";

const DOCK = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee";

interface PopoutRef {
  id: string;
  window: Window;
  group: GroupLike;
}

function fakeWindow(): Window {
  const target = new EventTarget();
  return {
    closed: false,
    document: document.implementation.createHTMLDocument("popout"),
    addEventListener: target.addEventListener.bind(target),
    removeEventListener: target.removeEventListener.bind(target),
    dispatchEvent: (event: Event) => target.dispatchEvent(event),
  } as unknown as Window;
}

function registry() {
  let onAdd: (popout: PopoutRef) => void = () => {};
  return {
    api: {
      onDidAddPopoutGroup(cb: (popout: PopoutRef) => void) {
        onAdd = cb;
        return { dispose() {} };
      },
      onDidRemovePopoutGroup() {
        return { dispose() {} };
      },
    },
    add(popout: PopoutRef) {
      onAdd(popout);
    },
  };
}

function popout(win: Window): PopoutRef {
  return {
    id: "group-1",
    window: win,
    group: {
      api: {
        id: "group-1",
        location: { type: "popout", popoutUrl: `/popout.html?dock=${DOCK}` },
        isVisible: true,
      },
      panels: [],
    },
  };
}

afterEach(() => {
  resetActivityForTests();
  resetWindowsForTests();
});

describe("lastUserDockId", () => {
  it("is the main window before anyone interacts", () => {
    expect(lastUserDockId()).toBeNull();
  });

  it("names the popout the user last used", () => {
    const events = registry();
    bindWindowRegistry(events.api);
    const win = fakeWindow();
    events.add(popout(win));
    noteUserWindow(win);
    expect(lastUserDockId()).toBe(DOCK);
  });

  it("falls back to the main window when that popout closed", () => {
    const events = registry();
    bindWindowRegistry(events.api);
    const win = fakeWindow();
    events.add(popout(win));
    noteUserWindow(win);
    (win as unknown as { closed: boolean }).closed = true;
    expect(lastUserDockId()).toBeNull();
  });

  it("ignores a window the registry does not know", () => {
    bindWindowRegistry(registry().api);
    noteUserWindow(fakeWindow());
    expect(lastUserDockId()).toBeNull();
  });
});

describe("bindWindowActivity", () => {
  it("follows a pointerdown in each registered window", () => {
    const events = registry();
    bindWindowRegistry(events.api);
    const win = fakeWindow();
    events.add(popout(win));
    const stop = bindWindowActivity();

    win.document.dispatchEvent(new Event("pointerdown"));
    expect(lastUserDockId()).toBe(DOCK);

    document.dispatchEvent(new Event("pointerdown"));
    expect(lastUserDockId()).toBeNull();

    stop();
  });

  it("stops reporting after it is released", () => {
    const events = registry();
    bindWindowRegistry(events.api);
    const win = fakeWindow();
    events.add(popout(win));
    const stop = bindWindowActivity();
    stop();
    win.document.dispatchEvent(new Event("pointerdown"));
    expect(lastUserDockId()).toBeNull();
  });
});

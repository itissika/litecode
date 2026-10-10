import { afterEach, describe, expect, it, vi } from "vitest";

import { useEditorStore } from "../../stores/editorStore";
import { bindWorkbenchKeys } from "./keys";
import { bindWindowRegistry, resetWindowsForTests } from "./windows";

const DOCK = "bbbbbbbb-cccc-4ddd-8eee-ffffffffffff";

function popoutEvents() {
  let onAdd: (popout: {
    id: string;
    window: Window;
    group: { api: { id: string; location: { type: string; popoutUrl: string } } };
  }) => void = () => {};
  let onRemove: (popout: { id: string; window: Window; group: { api: { id?: string } } }) => void =
    () => {};
  return {
    api: {
      onDidAddPopoutGroup(cb: typeof onAdd) {
        onAdd = cb;
        return { dispose() {} };
      },
      onDidRemovePopoutGroup(cb: typeof onRemove) {
        onRemove = cb;
        return { dispose() {} };
      },
    },
    add(popout: Parameters<typeof onAdd>[0]) {
      onAdd(popout);
    },
    remove(popout: Parameters<typeof onRemove>[0]) {
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
    location: { href: `/popout.html?dock=${DOCK}` },
    addEventListener: target.addEventListener.bind(target),
    removeEventListener: target.removeEventListener.bind(target),
    dispatchEvent: (event: Event) => target.dispatchEvent(event),
  } as unknown as Window;
}

afterEach(() => {
  resetWindowsForTests();
});

describe("bindWorkbenchKeys", () => {
  it("saves from a popout window and drops the listener when that window closes", () => {
    const save = vi.fn(async () => {});
    const previous = useEditorStore.getState().save;
    useEditorStore.setState({ save });
    const events = popoutEvents();
    bindWindowRegistry(events.api);
    const stop = bindWorkbenchKeys();
    const child = eventWindow();
    const item = {
      id: "group-1",
      window: child,
      group: {
        api: {
          id: "group-1",
          location: { type: "popout", popoutUrl: `/popout.html?dock=${DOCK}` },
        },
      },
    };
    events.add(item);
    child.dispatchEvent(new KeyboardEvent("keydown", { key: "s", ctrlKey: true }));
    expect(save).toHaveBeenCalledTimes(1);
    events.remove(item);
    child.dispatchEvent(new KeyboardEvent("keydown", { key: "s", ctrlKey: true }));
    expect(save).toHaveBeenCalledTimes(1);
    stop();
    useEditorStore.setState({ save: previous });
  });
});

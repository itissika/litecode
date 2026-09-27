import { afterEach, describe, expect, it } from "vitest";

import { layoutStore } from "./useDockviewConfig";

const KEY = "litecode-dockview-layout-v2";

function stubHost(host: unknown): void {
  Object.defineProperty(window, "litecode", { value: host, configurable: true });
}

afterEach(() => {
  Reflect.deleteProperty(window, "litecode");
  localStorage.clear();
});

describe("layoutStore", () => {
  it("uses the desktop host for a local workbench", () => {
    const saved: string[] = [];
    stubHost({
      getSessionMode: () => "local",
      loadLayout: () => '{"schemaVersion":3}',
      saveLayout: (payload: string) => saved.push(payload),
    });

    const store = layoutStore();
    expect(store.load()).toBe('{"schemaVersion":3}');
    store.save("payload");
    expect(saved).toEqual(["payload"]);
    expect(localStorage.getItem(KEY)).toBeNull();
  });

  it("keeps browser storage in a browser and in a remote workbench", () => {
    expect(layoutStore().load()).toBeNull();

    // Remote workbenches have a stable origin of their own, and a stale host
    // (no layout bridge yet) must not silently lose snapshots either.
    for (const host of [
      { getSessionMode: () => "remote", loadLayout: () => "from-host" },
      { getSessionMode: () => "local" },
    ]) {
      localStorage.setItem(KEY, "from-storage");
      stubHost(host);
      const store = layoutStore();
      expect(store.load()).toBe("from-storage");
      store.save("next");
      expect(localStorage.getItem(KEY)).toBe("next");
    }
  });
});

import { afterEach, describe, expect, it } from "vitest";

import { layoutStore, preventCrossZoneDrop } from "./useDockviewConfig";

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

describe("preventCrossZoneDrop", () => {
  function drop(
    source: "grid" | "edge" | "popout",
    target: "grid" | "edge" | "popout" | null,
  ): boolean {
    let prevented = false;
    const api = {
      getPanel: () => ({ api: { id: "panel", location: { type: source } } }),
      getGroup: () => ({ api: { location: { type: source } } }),
    } as unknown as Parameters<typeof preventCrossZoneDrop>[1];
    preventCrossZoneDrop(
      {
        getData: () => ({ panelId: "panel", groupId: "group" }),
        group: target
          ? { api: { location: { type: target } } }
          : undefined,
        preventDefault: () => {
          prevented = true;
        },
      } as unknown as Parameters<typeof preventCrossZoneDrop>[0],
      api,
    );
    return prevented;
  }

  it("keeps edge rails out of the center grid and the center out of the rails", () => {
    expect(drop("edge", "grid")).toBe(true);
    expect(drop("grid", "edge")).toBe(true);
    expect(drop("grid", "grid")).toBe(false);
    expect(drop("edge", "edge")).toBe(false);
    expect(drop("edge", null)).toBe(true);
    expect(drop("grid", null)).toBe(false);
  });

  it("lets a center tab move between the main grid and popout windows", () => {
    expect(drop("grid", "popout")).toBe(false);
    expect(drop("popout", "grid")).toBe(false);
    expect(drop("popout", "popout")).toBe(false);
    expect(drop("popout", null)).toBe(false);
    expect(drop("edge", "popout")).toBe(true);
    expect(drop("popout", "edge")).toBe(true);
  });
});

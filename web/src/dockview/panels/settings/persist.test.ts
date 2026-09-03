import { afterEach, describe, expect, it, vi } from "vitest";

import {
  SettingsPersistController,
  flushRegisteredSettings,
  registerSettingsFlush,
  shouldHydrateDraftFromStore,
  type PersistStatus,
} from "../../../lib/settingsPersist";

afterEach(() => {
  vi.useRealTimers();
});

function makeController(opts?: {
  serialize?: (d: string) => { ok: string } | { skip: "unchanged" | "invalid" };
  commit?: (p: string) => Promise<void>;
  revert?: () => void;
  debounceMs?: number;
}) {
  const statuses: PersistStatus[] = [];
  let snapshot = "a";
  const revert = opts?.revert ?? (() => {
    snapshot = "a";
  });
  const controller = new SettingsPersistController(snapshot, {
    debounceMs: opts?.debounceMs ?? 400,
    setStatus: (s) => statuses.push(s),
    serialize:
      opts?.serialize ??
      ((d) => {
        if (d === "invalid") return { skip: "invalid" };
        return { ok: d };
      }),
    commit: opts?.commit ?? (async () => undefined),
    revert: () => {
      revert();
      controller.schedule(snapshot);
    },
  });
  return { controller, statuses, getSnapshot: () => snapshot, setSnapshot: (v: string) => { snapshot = v; } };
}

describe("shouldHydrateDraftFromStore", () => {
  it("keeps incomplete local drafts instead of snapping back to the store", () => {
    expect(shouldHydrateDraftFromStore("invalid")).toBe(false);
    expect(shouldHydrateDraftFromStore("pending")).toBe(false);
    expect(shouldHydrateDraftFromStore("saving")).toBe(false);
    expect(shouldHydrateDraftFromStore("idle")).toBe(true);
    expect(shouldHydrateDraftFromStore("saved")).toBe(true);
    expect(shouldHydrateDraftFromStore("error")).toBe(true);
  });
});

describe("SettingsPersistController", () => {
  it("skips RPC when the payload is unchanged", async () => {
    const commit = vi.fn(async () => undefined);
    const { controller } = makeController({ commit });
    controller.schedule("a");
    await controller.flush();
    expect(commit).not.toHaveBeenCalled();
  });

  it("debounces and coalesces to the latest draft", async () => {
    vi.useFakeTimers();
    const commit = vi.fn(async () => undefined);
    const { controller } = makeController({ commit });
    controller.schedule("b");
    controller.schedule("c");
    expect(commit).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(400);
    expect(commit).toHaveBeenCalledTimes(1);
    expect(commit).toHaveBeenCalledWith("c");
  });

  it("does not PUT invalid payloads", async () => {
    const commit = vi.fn(async () => undefined);
    const { controller, statuses } = makeController({ commit });
    controller.schedule("invalid");
    await controller.flush();
    expect(commit).not.toHaveBeenCalled();
    expect(statuses).toContain("invalid");
  });

  it("does not revert local drafts when flushing an invalid payload", async () => {
    const revert = vi.fn();
    const { controller, getSnapshot } = makeController({ revert });
    controller.schedule("invalid");
    await controller.flush();
    expect(revert).not.toHaveBeenCalled();
    expect(getSnapshot()).toBe("a");
  });

  it("reverts to the snapshot when commit fails", async () => {
    vi.useFakeTimers();
    const commit = vi.fn(async () => {
      throw new Error("nope");
    });
    let live = "a";
    const statuses: PersistStatus[] = [];
    const controller = new SettingsPersistController(live, {
      debounceMs: 0,
      setStatus: (s) => statuses.push(s),
      serialize: (d) => ({ ok: d }),
      commit,
      revert: () => {
        live = "a";
      },
    });
    controller.schedule("b");
    await vi.advanceTimersByTimeAsync(0);
    await Promise.resolve();
    expect(live).toBe("a");
    expect(statuses.at(-1)).toBe("error");
  });

  it("does not apply a stale commit over a newer draft", async () => {
    vi.useFakeTimers();
    let resolveFirst: (() => void) | undefined;
    const commit = vi.fn((payload: string) => {
      if (payload === "b") {
        return new Promise<void>((resolve) => {
          resolveFirst = resolve;
        });
      }
      return Promise.resolve();
    });
    const { controller } = makeController({ commit, debounceMs: 0 });
    controller.schedule("b");
    await vi.advanceTimersByTimeAsync(0);
    controller.schedule("c");
    resolveFirst?.();
    await vi.advanceTimersByTimeAsync(0);
    await Promise.resolve();
    await Promise.resolve();
    expect(commit.mock.calls.map((c) => c[0])).toEqual(["b", "c"]);
  });

  it("treats object key order as the same payload", async () => {
    const commit = vi.fn(async () => undefined);
    const statuses: PersistStatus[] = [];
    const controller = new SettingsPersistController(
      { b: 1, a: 1 },
      {
        debounceMs: 0,
        setStatus: (s) => statuses.push(s),
        serialize: (d) => ({ ok: d }),
        commit,
        revert: () => undefined,
      },
    );
    controller.schedule({ a: 1, b: 1 });
    await controller.flush();
    expect(commit).not.toHaveBeenCalled();
  });

  it("does not re-PUT after commit when keys are reshuffled", async () => {
    const commit = vi.fn(async () => undefined);
    const controller = new SettingsPersistController<Record<string, number>, Record<string, number>>(
      { a: 1 },
      {
        debounceMs: 0,
        setStatus: () => undefined,
        serialize: (d) => ({ ok: d }),
        commit,
        revert: () => undefined,
      },
    );
    controller.schedule({ z: 2, a: 1 });
    await controller.flush();
    expect(commit).toHaveBeenCalledTimes(1);
    controller.schedule({ a: 1, z: 2 });
    await controller.flush();
    expect(commit).toHaveBeenCalledTimes(1);
  });

  it("does not spin a zero-debounce loop while a commit is in flight", async () => {
    vi.useFakeTimers();
    const commit = vi.fn(async () => undefined);
    const { controller } = makeController({ commit, debounceMs: 0 });
    controller.schedule("b");
    await vi.advanceTimersByTimeAsync(0);
    controller.schedule("b");
    await vi.advanceTimersByTimeAsync(0);
    expect(commit).toHaveBeenCalledTimes(1);
  });
});

describe("flushRegisteredSettings", () => {
  it("runs the registered flush before resolving", async () => {
    const flush = vi.fn(async () => undefined);
    const unreg = registerSettingsFlush(flush);
    await flushRegisteredSettings();
    expect(flush).toHaveBeenCalledTimes(1);
    unreg();
    await flushRegisteredSettings();
    expect(flush).toHaveBeenCalledTimes(1);
  });

  it("runs every registered flush", async () => {
    const a = vi.fn(async () => undefined);
    const b = vi.fn(async () => undefined);
    const ua = registerSettingsFlush(a);
    const ub = registerSettingsFlush(b);
    await flushRegisteredSettings();
    expect(a).toHaveBeenCalledTimes(1);
    expect(b).toHaveBeenCalledTimes(1);
    ua();
    ub();
  });
});

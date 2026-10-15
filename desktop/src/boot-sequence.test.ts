import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { BootAbortedError, isBootAborted, runBootSequence, type BootSequenceDeps } from "./boot-sequence";

class FakeWindow {
  destroyed = false;
  private listeners: Array<() => void> = [];
  isDestroyed(): boolean {
    return this.destroyed;
  }
  once(_event: "closed", listener: () => void): void {
    this.listeners.push(listener);
  }
  /** User close (Alt+F4) or programmatic destroy: both emit "closed". */
  close(): void {
    if (this.destroyed) return;
    this.destroyed = true;
    const pending = this.listeners;
    this.listeners = [];
    for (const l of pending) l();
  }
}

type Handle = { id: string; stopped: boolean };

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

const tick = () => new Promise((r) => setImmediate(r));

function harness(opts: { prepareError?: Error } = {}) {
  const log: string[] = [];
  const ready = deferred<Handle>();
  const win = new FakeWindow();
  const handle: Handle = { id: "sidecar", stopped: false };
  let abortCleanup: Promise<void> | null = null;
  const killed = { value: false };
  const deps: BootSequenceDeps<FakeWindow, Handle> = {
    // Mirrors startSidecar({ signal }): abort kills the child and rejects with the reason.
    startBackend: (signal) => {
      log.push("startBackend");
      signal.addEventListener("abort", () => {
        killed.value = true;
        log.push("killChild");
        ready.reject(signal.reason);
      });
      return ready.promise;
    },
    createSkeleton: async () => {
      log.push("createSkeleton");
      return win;
    },
    onBackend: () => log.push("onBackend"),
    prepare: async (h) => {
      log.push(`prepare:${h.id}`);
      if (opts.prepareError) throw opts.prepareError;
      return "http://127.0.0.1:1234/";
    },
    loadWorkbench: async (w, url) => {
      assert.equal(w, win, "workbench loads into the skeleton window");
      log.push(`load:${url}`);
    },
    stopBackend: async (h) => {
      h.stopped = true;
      log.push("stopBackend");
    },
    closeSkeleton: (w) => {
      log.push("closeSkeleton");
      w.close();
    },
    recordRecent: () => log.push("recordRecent"),
    onAbort: (cleanup) => {
      abortCleanup = cleanup;
    },
  };
  return { log, ready, win, handle, deps, killed, abortCleanup: () => abortCleanup };
}

describe("boot sequence (WP-169 S1/S1b/S2)", () => {
  it("creates the skeleton before the sidecar is ready, loads after READY/health, records recents once after load", async () => {
    const h = harness();
    const run = runBootSequence(h.deps);
    await tick();
    assert.deepEqual(h.log, ["startBackend", "createSkeleton"], "skeleton exists while READY is pending");
    h.ready.resolve(h.handle);
    const win = await run;
    assert.equal(win, h.win);
    assert.deepEqual(h.log, [
      "startBackend",
      "createSkeleton",
      "onBackend",
      "prepare:sidecar",
      "load:http://127.0.0.1:1234/",
      "recordRecent",
    ]);
    assert.equal(h.log.filter((e) => e === "recordRecent").length, 1);
  });

  it("closes the skeleton and rethrows the original error when the sidecar fails to start", async () => {
    const h = harness();
    const run = runBootSequence(h.deps);
    await tick();
    const boom = new Error("sidecar timed out waiting for LITECODE_READY (90000ms)");
    h.ready.reject(boom);
    await assert.rejects(run, (err) => err === boom);
    assert.ok(h.win.destroyed);
    assert.ok(h.log.includes("closeSkeleton"));
    assert.ok(!h.log.includes("recordRecent"));
    assert.ok(!h.log.some((e) => e.startsWith("load:")));
    assert.equal(h.abortCleanup(), null, "a failed boot is not treated as a user close");
  });

  it("closes the skeleton and rethrows when the attach/health check fails", async () => {
    const mismatch = new Error("Sidecar health reports a different workspace");
    const h = harness({ prepareError: mismatch });
    const run = runBootSequence(h.deps);
    h.ready.resolve(h.handle);
    await assert.rejects(run, (err) => err === mismatch);
    assert.ok(h.win.destroyed);
    assert.ok(!h.log.includes("recordRecent"));
  });

  it("remote branch: health gates loadURL(handshake) into the same window", async () => {
    const log: string[] = [];
    const health = deferred<null>();
    const win = new FakeWindow();
    const run = runBootSequence<FakeWindow, null>({
      startBackend: () => {
        log.push("health");
        return health.promise;
      },
      createSkeleton: async () => {
        log.push("createSkeleton");
        return win;
      },
      prepare: async () => "https://remote.example/#token",
      loadWorkbench: async (w, url) => {
        assert.equal(w, win);
        log.push(`load:${url}`);
      },
      stopBackend: async () => undefined,
      closeSkeleton: (w) => w.close(),
    });
    await tick();
    assert.deepEqual(log, ["health", "createSkeleton"]);
    health.resolve(null);
    await run;
    assert.deepEqual(log, ["health", "createSkeleton", "load:https://remote.example/#token"]);
  });

  it("remote branch: health failure closes the skeleton and rethrows", async () => {
    const win = new FakeWindow();
    const boom = new Error("health timed out");
    const run = runBootSequence<FakeWindow, null>({
      startBackend: async () => {
        throw boom;
      },
      createSkeleton: async () => win,
      prepare: async () => "x",
      loadWorkbench: async () => undefined,
      stopBackend: async () => undefined,
      closeSkeleton: (w) => w.close(),
    });
    await assert.rejects(run, (err) => err === boom);
    assert.ok(win.destroyed);
  });

  it("user closes the skeleton while the sidecar is still starting: aborts via the signal, child killed at once", async () => {
    const h = harness();
    const run = runBootSequence(h.deps);
    await tick();
    h.win.close(); // Alt+F4 / taskbar close
    await assert.rejects(run, (err) => isBootAborted(err) && err instanceof BootAbortedError);
    assert.equal(h.killed.value, true, "child killed immediately, no wait for READY");
    const cleanup = h.abortCleanup();
    assert.ok(cleanup, "abort cleanup is handed to the quit path");
    await cleanup; // settles without READY ever arriving
    assert.ok(!h.log.includes("stopBackend"), "nothing left to stop");
    assert.ok(!h.log.includes("closeSkeleton"), "no failure handling");
    assert.ok(!h.log.some((e) => e.startsWith("load:")));
    assert.ok(!h.log.includes("recordRecent"));
  });

  it("user closes the skeleton after READY (during health): stops the running sidecar", async () => {
    const h = harness();
    const gate = deferred<string>();
    h.deps.prepare = async () => gate.promise;
    const run = runBootSequence(h.deps);
    h.ready.resolve(h.handle);
    await tick();
    h.win.close();
    await assert.rejects(run, isBootAborted);
    await h.abortCleanup();
    assert.equal(h.handle.stopped, true);
    gate.resolve("http://127.0.0.1:1/");
    await tick();
    assert.ok(!h.log.some((e) => e.startsWith("load:")));
  });

  it("successful boot never aborts the backend signal", async () => {
    const h = harness();
    const run = runBootSequence(h.deps);
    h.ready.resolve(h.handle);
    await run;
    assert.equal(h.killed.value, false);
    assert.equal(h.handle.stopped, false);
  });
});

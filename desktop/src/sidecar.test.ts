import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { describe, it } from "node:test";
import type { SidecarHandle } from "./sidecar";

import { formatSidecarBootFailure, stopSidecar, waitForReady } from "./sidecar";

function longRunningChild(): SidecarHandle {
  const child = spawn(
    process.execPath,
    ["-e", "setInterval(() => {}, 1000)"],
    { stdio: ["ignore", "ignore", "ignore"], windowsHide: true },
  );
  const handle: SidecarHandle = {
    process: child,
    readyUrl: "http://127.0.0.1:0/",
    token: "",
    workspace: "",
    intentionalStop: false,
    markIntentionalStop() {
      handle.intentionalStop = true;
    },
  };
  return handle;
}

describe("formatSidecarBootFailure", () => {
  it("includes key fields code, bin, and log", () => {
    const text = formatSidecarBootFailure({
      bin: "E:\\litecode\\dist\\product\\litecode.exe",
      workspace: "E:\\proj",
      code: 1,
      output: "Error: cannot open sessions.db at E:\\proj\\.litecode\\sessions.db",
    });
    assert.match(text, /code=1/);
    assert.match(text, /binary: E:\\litecode\\dist\\product\\litecode\.exe/);
    assert.match(text, /log file: /);
  });

  it("notes when sidecar printed nothing", () => {
    const text = formatSidecarBootFailure({
      bin: "/bin/litecode",
      workspace: "/tmp/ws",
      code: 1,
      output: "  ",
    });
    assert.match(text, /produced no stdout\/stderr/);
  });
});

describe("stopSidecar (DESK-04 sidecar cleanup)", () => {
  it("terminates a running sidecar child", async () => {
    const handle = longRunningChild();
    assert.equal(handle.process.exitCode, null, "child should be running");
    await stopSidecar(handle);
    assert.ok(handle.process.exitCode !== null || handle.process.signalCode !== null, "child must be terminated");
  });

  it("is a no-op for a null handle", async () => {
    await assert.doesNotReject(stopSidecar(null));
  });
});

describe("stopSidecar intentional stop (D2)", () => {
  it("marks intentionalStop before killing", async () => {
    const handle = longRunningChild();
    assert.equal(handle.intentionalStop, false);
    await stopSidecar(handle);
    assert.equal(handle.intentionalStop, true);
  });
});

function pipedChild(script: string) {
  return spawn(process.execPath, ["-e", script], {
    stdio: ["ignore", "pipe", "pipe"],
    windowsHide: true,
  });
}

const CTX = { bin: "fake-litecode", workspace: "/tmp/ws" };

describe("startSidecar abort signal (WP-169)", () => {
  it("abort before READY kills the child and rejects with signal.reason", async () => {
    const child = pipedChild("setInterval(() => {}, 1000)"); // never prints READY
    const controller = new AbortController();
    const reason = new Error("boot aborted: window closed");
    const ready = waitForReady(child, 90_000, CTX, controller.signal);
    const exited = new Promise<void>((resolve) => child.once("exit", () => resolve()));
    setTimeout(() => controller.abort(reason), 50);
    await assert.rejects(ready, (err) => err === reason);
    await exited;
    assert.ok(child.exitCode !== null || child.signalCode !== null, "child must be killed");
  });

  it("an already-aborted signal kills the child at once", async () => {
    const child = pipedChild("setInterval(() => {}, 1000)");
    const controller = new AbortController();
    const reason = new Error("already aborted");
    controller.abort(reason);
    const exited = new Promise<void>((resolve) => child.once("exit", () => resolve()));
    await assert.rejects(waitForReady(child, 90_000, CTX, controller.signal), (err) => err === reason);
    await exited;
  });

  it("without a signal: unchanged, resolves on LITECODE_READY", async () => {
    const child = pipedChild(
      "console.log('booting'); console.log('LITECODE_READY http://127.0.0.1:4567'); setInterval(() => {}, 1000)",
    );
    try {
      const url = await waitForReady(child, 10_000, CTX);
      assert.equal(url, "http://127.0.0.1:4567/");
    } finally {
      child.kill();
    }
  });

  it("READY settles first: a later abort does nothing (listener removed)", async () => {
    const child = pipedChild("console.log('LITECODE_READY http://127.0.0.1:4568/'); setInterval(() => {}, 1000)");
    const controller = new AbortController();
    try {
      const url = await waitForReady(child, 10_000, CTX, controller.signal);
      assert.equal(url, "http://127.0.0.1:4568/");
      controller.abort(new Error("late"));
      await new Promise((r) => setTimeout(r, 50));
      assert.equal(child.exitCode, null, "a late abort must not kill a ready sidecar");
    } finally {
      child.kill();
    }
  });
});

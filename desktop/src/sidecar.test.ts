import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { describe, it } from "node:test";
import type { SidecarHandle } from "./sidecar";

import { stopSidecar } from "./sidecar";

function longRunningChild(): SidecarHandle {
  const child = spawn(
    process.execPath,
    ["-e", "setInterval(() => {}, 1000)"],
    { stdio: ["ignore", "ignore", "ignore"], windowsHide: true },
  );
  return {
    process: child,
    readyUrl: "http://127.0.0.1:0/",
    token: "",
    workspace: "",
  };
}

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

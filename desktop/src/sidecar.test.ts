import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { describe, it } from "node:test";
import type { SidecarHandle } from "./sidecar";

import { formatSidecarBootFailure, stopSidecar } from "./sidecar";

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

describe("formatSidecarBootFailure", () => {
  it("includes exit code, paths, and sidecar output", () => {
    const text = formatSidecarBootFailure({
      bin: "E:\\litecode\\dist\\product\\litecode.exe",
      workspace: "E:\\proj",
      code: 1,
      output: "Error: cannot open sessions.db at E:\\proj\\.litecode\\sessions.db",
    });
    assert.match(text, /code=1/);
    assert.match(text, /binary: /);
    assert.match(text, /workspace: /);
    assert.match(text, /log file: /);
    assert.match(text, /cannot open sessions\.db/);
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

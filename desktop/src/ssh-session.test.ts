import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  buildScpUploadArgs,
  buildSshArgs,
  posixShellQuote,
  relativeHomePath,
  validateSshTarget,
} from "./ssh-session";

describe("ssh-session safety helpers", () => {
  const config = {
    target: {
      host: "dev-box",
      user: "alice",
      port: 2222,
      identityFile: "C:\\Users\\alice\\.ssh\\id_ed25519",
    },
  };

  it("constructs SSH arguments without a shell", () => {
    assert.deepEqual(buildSshArgs(config, "sh -lc 'echo ok'"), [
      "-o", "BatchMode=yes",
      "-o", "ConnectTimeout=15",
      "-p", "2222",
      "-i", "C:\\Users\\alice\\.ssh\\id_ed25519",
      "-l", "alice",
      "dev-box",
      "sh -lc 'echo ok'",
    ]);
  });

  it("formats SCP remote destinations without shell quotes", () => {
    const args = buildScpUploadArgs(
      { target: { host: "2001:db8::10", user: "alice" } },
      "artifact.tar",
      "/home/alice/.litecode-upload-deadbeef.tar",
    );
    assert.equal(args.at(-1), "alice@[2001:db8::10]:/home/alice/.litecode-upload-deadbeef.tar");
  });

  it("escapes spaces in SCP remote paths", () => {
    const args = buildScpUploadArgs(
      { target: { host: "dev-box", user: "alice" } },
      "artifact.tar",
      "/home/alice/a folder/artifact.tar",
    );
    assert.equal(args.at(-1), "alice@dev-box:/home/alice/a\\ folder/artifact.tar");
  });

  it("adds -r for recursive directory uploads", () => {
    const args = buildScpUploadArgs(
      { target: { host: "dev-box", user: "alice" } },
      "C:\\models\\granite",
      "/home/alice/.litecode/models/ibm-granite/granite-embedding-97m-multilingual-r2",
      { recursive: true },
    );
    assert.ok(args.includes("-r"));
  });

  it("rejects ambiguous targets and paths", () => {
    assert.throws(() => validateSshTarget({ host: "-oProxyCommand=bad" }));
    assert.throws(() => validateSshTarget({ host: "alice@example.test" }));
    assert.throws(() => relativeHomePath("../outside"));
    assert.throws(() => relativeHomePath("/etc"));
  });

  it("uses POSIX-safe single-quote escaping", () => {
    assert.equal(posixShellQuote("it's safe"), "'it'\"'\"'s safe'");
  });
});

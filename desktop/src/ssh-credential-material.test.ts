import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { describe, it } from "node:test";

import {
  materializePasswordAskPass,
  materializePrivateKeyFile,
  readPrivateKeyFile,
  removeMaterializedKey,
  withMaterializedKeyDirectory,
} from "./ssh-credential-material";

describe("materializePasswordAskPass", () => {
  it("writes a plaintext password file and an askpass helper in a temp dir", () => {
    const m = materializePasswordAskPass("s3cret");
    try {
      assert.ok(fs.existsSync(m.keyDirectory));
      assert.equal(fs.readFileSync(path.join(m.keyDirectory, "password"), "utf8"), "s3cret");
      assert.equal(fs.existsSync(m.askPassCommand), true);
    } finally {
      removeMaterializedKey(m.keyDirectory);
    }
  });
});

describe("materializePrivateKeyFile", () => {
  it("writes the key material into a temp identity file", () => {
    const m = materializePrivateKeyFile("-----BEGIN OPENSSH PRIVATE KEY-----");
    try {
      assert.equal(fs.readFileSync(m.keyFile, "utf8"), "-----BEGIN OPENSSH PRIVATE KEY-----");
    } finally {
      removeMaterializedKey(m.keyDirectory);
    }
  });
});

describe("removeMaterializedKey", () => {
  it("recursively removes the temp directory", () => {
    const m = materializePasswordAskPass("x");
    assert.ok(fs.existsSync(m.keyDirectory));
    removeMaterializedKey(m.keyDirectory);
    assert.equal(fs.existsSync(m.keyDirectory), false);
  });

  it("is a no-op for an empty key directory", () => {
    assert.doesNotThrow(() => removeMaterializedKey());
  });
});

describe("withMaterializedKeyDirectory (DESK-03 finally cleanup)", () => {
  it("removes the temp dir even when the body throws", async () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "litecode-cred-test-"));
    fs.writeFileSync(path.join(dir, "password"), "s3cret", "utf8");
    await assert.rejects(
      withMaterializedKeyDirectory(dir, () => {
        throw new Error("boom");
      }),
      /boom/,
    );
    assert.equal(fs.existsSync(dir), false, "temp dir must be cleaned up on error");
  });

  it("removes the temp dir on a normal return", async () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "litecode-cred-test-"));
    const result = await withMaterializedKeyDirectory(dir, async () => 42);
    assert.equal(result, 42);
    assert.equal(fs.existsSync(dir), false);
  });
});

describe("readPrivateKeyFile (DESK-02 path control)", () => {
  it("reads a regular private key file", () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "litecode-key-test-"));
    const file = path.join(dir, "id_ed25519");
    fs.writeFileSync(file, "-----BEGIN OPENSSH PRIVATE KEY-----", "utf8");
    try {
      assert.equal(readPrivateKeyFile(file), "-----BEGIN OPENSSH PRIVATE KEY-----");
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });

  it("rejects a directory path", () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "litecode-key-test-"));
    try {
      assert.throws(() => readPrivateKeyFile(dir), /regular file/);
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });

  it("rejects a missing path", () => {
    assert.throws(() => readPrivateKeyFile(path.join(os.tmpdir(), "definitely-missing-key")), /not readable/);
  });

  it("rejects empty or newline-containing paths", () => {
    assert.throws(() => readPrivateKeyFile(""), /non-empty/);
    assert.throws(() => readPrivateKeyFile("a\nb"), /non-empty/);
  });
});

import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { afterEach, describe, it } from "node:test";

import {
  assertDevUiConfigured,
  devUiDocumentUrl,
  sameWorkbenchDocument,
  syncDevUpstream,
} from "./dev-ui";

const dirs: string[] = [];

afterEach(() => {
  for (const dir of dirs.splice(0)) {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

function tempDir(): string {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "litecode-dev-ui-"));
  dirs.push(dir);
  return dir;
}

describe("devUiDocumentUrl", () => {
  it("is unset when the env is blank", () => {
    assert.equal(devUiDocumentUrl(undefined), null);
    assert.equal(devUiDocumentUrl("  "), null);
  });

  it("normalizes a loopback origin", () => {
    assert.equal(devUiDocumentUrl("http://127.0.0.1:5179"), "http://127.0.0.1:5179/");
    assert.equal(devUiDocumentUrl("http://127.0.0.1:5179/"), "http://127.0.0.1:5179/");
    assert.equal(devUiDocumentUrl("http://localhost:5179/"), "http://localhost:5179/");
  });

  it("rejects anything that is not a bare loopback origin", () => {
    assert.throws(() => devUiDocumentUrl("http://127.0.0.1:5179/index.html"), /origin/);
    assert.throws(() => devUiDocumentUrl("http://127.0.0.1:5179/?token=x"), /origin/);
    assert.throws(() => devUiDocumentUrl("http://127.0.0.1:5179/#x"), /origin/);
    assert.throws(() => devUiDocumentUrl("http://user:pass@127.0.0.1:5179/"), /credentials/);
    assert.throws(() => devUiDocumentUrl("file:///c:/web/index.html"), /http/);
    assert.throws(() => devUiDocumentUrl("http://example.com:5179/"), /loopback/);
    assert.throws(() => devUiDocumentUrl("not a url"), /not a URL/);
  });
});

describe("assertDevUiConfigured", () => {
  it("requires the upstream file whenever the dev document is set", () => {
    assert.doesNotThrow(() => assertDevUiConfigured(null, null));
    assert.doesNotThrow(() => assertDevUiConfigured(null, "C:\\upstream.txt"));
    assert.throws(
      () => assertDevUiConfigured("http://127.0.0.1:5179/", null),
      /LITECODE_DEV_UPSTREAM_FILE/,
    );
  });
});

describe("syncDevUpstream", () => {
  it("writes the sidecar origin and clears it", () => {
    const file = path.join(tempDir(), "upstream.txt");
    const env = { LITECODE_DEV_UPSTREAM_FILE: file };
    syncDevUpstream("http://127.0.0.1:4000/", env);
    assert.equal(fs.readFileSync(file, "utf8"), "http://127.0.0.1:4000");
    syncDevUpstream(null, env);
    assert.equal(fs.readFileSync(file, "utf8"), "");
  });

  it("does nothing when the dev loop did not set a file", () => {
    assert.doesNotThrow(() => syncDevUpstream("http://127.0.0.1:1/", {}));
  });

  it("refuses a non-loopback sidecar URL", () => {
    const file = path.join(tempDir(), "upstream.txt");
    assert.throws(
      () => syncDevUpstream("http://example.com:1/", { LITECODE_DEV_UPSTREAM_FILE: file }),
      /loopback/,
    );
    assert.equal(fs.existsSync(file), false);
  });
});

describe("sameWorkbenchDocument", () => {
  it("matches origin and path, ignoring query", () => {
    assert.equal(
      sameWorkbenchDocument("http://127.0.0.1:5179/?x=1", "http://127.0.0.1:5179/"),
      true,
    );
    assert.equal(
      sameWorkbenchDocument("http://127.0.0.1:5179/", "http://127.0.0.1:4000/"),
      false,
    );
    assert.equal(sameWorkbenchDocument("", "http://127.0.0.1:5179/"), false);
  });
});

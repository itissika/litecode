import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { describe, it } from "node:test";

import { normalizeWorkspace, stripVerbatimLap } from "./lap-path";

describe("stripVerbatimLap", () => {
  it("strips drive verbatim and uppercases drive", () => {
    assert.equal(stripVerbatimLap("\\\\?\\e:\\litecode"), "E:\\litecode");
    assert.equal(stripVerbatimLap("c:\\foo"), "C:\\foo");
  });

  it("strips UNC verbatim to \\\\host\\share form", () => {
    assert.equal(
      stripVerbatimLap("\\\\?\\UNC\\host\\share\\proj"),
      "\\\\host\\share\\proj",
    );
  });
});

describe("normalizeWorkspace", () => {
  it("resolves relative paths", () => {
    const abs = normalizeWorkspace(".");
    assert.ok(path.isAbsolute(abs));
    assert.ok(!abs.startsWith("\\\\?\\"));
  });

  it("realpaths an existing directory without verbatim prefix", () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "litecode-lap-"));
    try {
      const lap = normalizeWorkspace(dir);
      assert.ok(!lap.startsWith("\\\\?\\"), lap);
      assert.equal(lap, stripVerbatimLap(fs.realpathSync(dir)));
      // Drive letter policy
      if (/^[a-zA-Z]:/.test(lap)) {
        assert.equal(lap[0], lap[0]!.toUpperCase());
      }
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });
});

import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { describe, it } from "node:test";

import {
  layoutFilePath,
  readWorkspaceLayout,
  writeWorkspaceLayout,
} from "./layout-file";

function temporaryDir(): string {
  return fs.mkdtempSync(path.join(os.tmpdir(), "litecode-layout-"));
}

describe("workspace layout file", () => {
  it("round-trips a snapshot per workspace", () => {
    const root = temporaryDir();
    const first = path.join(root, "first");
    const second = path.join(root, "second");

    writeWorkspaceLayout(root, first, '{"schemaVersion":3}');
    assert.equal(readWorkspaceLayout(root, first), '{"schemaVersion":3}');

    // A different workspace stays empty — no cross-workspace bleed.
    assert.equal(readWorkspaceLayout(root, second), null);
    writeWorkspaceLayout(root, second, '{"schemaVersion":3,"layout":{}}');
    assert.equal(readWorkspaceLayout(root, first), '{"schemaVersion":3}');
  });

  it("reads null without a workspace or a saved file", () => {
    const root = temporaryDir();
    assert.equal(readWorkspaceLayout(root, null), null);
    assert.equal(readWorkspaceLayout(root, path.join(root, "never-saved")), null);
    assert.equal(
      fs.existsSync(layoutFilePath(root, path.join(root, "never-saved"))),
      false,
    );
  });

  it("ignores non-string and oversized payloads", () => {
    const root = temporaryDir();
    const workspace = path.join(root, "ws");

    writeWorkspaceLayout(root, workspace, { not: "a string" });
    writeWorkspaceLayout(root, workspace, "");
    writeWorkspaceLayout(root, workspace, "x".repeat(1_000_001));
    assert.equal(readWorkspaceLayout(root, workspace), null);
  });

  it("ignores writes without a workspace", () => {
    const root = temporaryDir();
    writeWorkspaceLayout(root, null, "{}");
    assert.equal(fs.existsSync(path.join(root, "layouts")), false);
  });
});

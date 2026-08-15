import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { describe, it } from "node:test";

import { RecentWorkspaces } from "./recents";

function temporaryStore(): { root: string; file: string; store: RecentWorkspaces } {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "litecode-recents-"));
  const file = path.join(root, "recent-workspaces.json");
  return { root, file, store: new RecentWorkspaces(file) };
}

describe("RecentWorkspaces", () => {
  it("retains pins while moving opened workspaces to the front", () => {
    const { root, store } = temporaryStore();
    const first = path.join(root, "first");
    const second = path.join(root, "second");
    fs.mkdirSync(first);
    fs.mkdirSync(second);
    try {
      store.record(first);
      store.setPinned(first, true);
      store.record(second);
      const rows = store.list();
      assert.equal(rows.length, 2);
      assert.equal(rows[0]!.path, path.resolve(first));
      assert.equal(rows[0]!.pinned, true);
      assert.equal(rows[1]!.path, path.resolve(second));
    } finally {
      fs.rmSync(root, { recursive: true, force: true });
    }
  });

  it("recovers from malformed persisted data", () => {
    const { root, file, store } = temporaryStore();
    try {
      fs.writeFileSync(file, "{ not json", "utf8");
      assert.deepEqual(store.list(), []);
      assert.ok(fs.readdirSync(root).some((name) => name.startsWith("recent-workspaces.json.corrupt-")));
    } finally {
      fs.rmSync(root, { recursive: true, force: true });
    }
  });
});

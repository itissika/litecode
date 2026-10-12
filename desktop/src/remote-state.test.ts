import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { clearManagedRemoteIfSession } from "./remote-state";

describe("clearManagedRemoteIfSession (D8)", () => {
  it("nulls when the handle still points at the failed session", () => {
    const session = { id: "a" };
    const other = { id: "b" };
    const current = { session, tunnel: 1 };
    assert.equal(clearManagedRemoteIfSession(current, session), null);
    assert.deepEqual(clearManagedRemoteIfSession(current, other), current);
    assert.equal(clearManagedRemoteIfSession(null, session), null);
  });
});

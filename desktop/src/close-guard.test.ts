import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { closeDecision } from "./close-guard";

const idle = {
  sessionsActive: false,
  confirmed: false,
  promptOpen: false,
  quitting: false,
};

describe("closeDecision", () => {
  it("closes straight away when nothing is running", () => {
    assert.equal(closeDecision(idle), "close");
    assert.equal(closeDecision({ ...idle, promptOpen: true }), "close");
  });

  it("asks before closing a workspace with a running session", () => {
    assert.equal(closeDecision({ ...idle, sessionsActive: true }), "prompt");
  });

  it("does not stack a second dialog over the first", () => {
    assert.equal(
      closeDecision({ ...idle, sessionsActive: true, promptOpen: true }),
      "wait",
    );
  });

  it("closes once the person confirmed, and during quit cleanup", () => {
    assert.equal(
      closeDecision({ ...idle, sessionsActive: true, confirmed: true }),
      "close",
    );
    assert.equal(
      closeDecision({ ...idle, sessionsActive: true, quitting: true }),
      "close",
    );
  });
});

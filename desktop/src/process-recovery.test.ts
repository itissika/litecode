import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  DEFAULT_SIDECAR_RECOVERY,
  DEFAULT_TUNNEL_RECOVERY,
  RecoveryCounter,
  recoveryDelayMs,
  shouldAutoRecover,
} from "./process-recovery";

describe("process-recovery", () => {
  it("never recovers intentional stops", () => {
    assert.equal(shouldAutoRecover(true, 0, DEFAULT_TUNNEL_RECOVERY), false);
  });

  it("allows limited unexpected recoveries", () => {
    assert.equal(shouldAutoRecover(false, 0, DEFAULT_TUNNEL_RECOVERY), true);
    assert.equal(shouldAutoRecover(false, 2, DEFAULT_TUNNEL_RECOVERY), true);
    assert.equal(shouldAutoRecover(false, 3, DEFAULT_TUNNEL_RECOVERY), false);
  });

  it("picks backoff delays by attempt", () => {
    assert.equal(recoveryDelayMs(1, DEFAULT_TUNNEL_RECOVERY), 1_000);
    assert.equal(recoveryDelayMs(2, DEFAULT_TUNNEL_RECOVERY), 2_000);
    assert.equal(recoveryDelayMs(9, DEFAULT_TUNNEL_RECOVERY), 4_000);
    assert.equal(recoveryDelayMs(1, DEFAULT_SIDECAR_RECOVERY), 800);
  });

  it("counts unexpected exits until the policy is exhausted", () => {
    const counter = new RecoveryCounter();
    assert.equal(counter.recordUnexpected(DEFAULT_SIDECAR_RECOVERY), true);
    assert.equal(counter.recordUnexpected(DEFAULT_SIDECAR_RECOVERY), true);
    assert.equal(counter.recordUnexpected(DEFAULT_SIDECAR_RECOVERY), false);
    counter.reset();
    assert.equal(counter.count, 0);
    assert.equal(counter.recordUnexpected(DEFAULT_SIDECAR_RECOVERY), true);
  });
});

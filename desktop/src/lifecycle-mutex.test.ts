import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { LifecycleMutex, SingleFlight } from "./lifecycle-mutex";

describe("LifecycleMutex", () => {
  it("runs jobs strictly in order", async () => {
    const mutex = new LifecycleMutex();
    const order: number[] = [];
    const a = mutex.run(async () => {
      await new Promise((r) => setTimeout(r, 30));
      order.push(1);
      return "a";
    });
    const b = mutex.run(async () => {
      order.push(2);
      return "b";
    });
    assert.deepEqual(await Promise.all([a, b]), ["a", "b"]);
    assert.deepEqual(order, [1, 2]);
  });

  it("queues the next job even when the previous one rejects", async () => {
    const mutex = new LifecycleMutex();
    const failed = mutex.run(async () => {
      throw new Error("boom");
    });
    await assert.rejects(failed, /boom/);
    const ok = await mutex.run(async () => "recovered");
    assert.equal(ok, "recovered");
  });
});

describe("SingleFlight", () => {
  it("rejects a concurrent caller by default", async () => {
    const flight = new SingleFlight<string>();
    let release!: () => void;
    const gate = new Promise<void>((r) => {
      release = r;
    });
    const first = flight.run(async () => {
      await gate;
      return "one";
    });
    await assert.rejects(flight.run(async () => "two"), /already in progress/i);
    release();
    assert.equal(await first, "one");
    assert.equal(flight.busy, false);
  });

  it("reuses the in-flight promise when mode is reuse", async () => {
    const flight = new SingleFlight<string>();
    let release!: () => void;
    const gate = new Promise<void>((r) => {
      release = r;
    });
    let runs = 0;
    const first = flight.run(async () => {
      runs += 1;
      await gate;
      return "shared";
    }, "reuse");
    const second = flight.run(async () => {
      runs += 1;
      return "other";
    }, "reuse");
    release();
    assert.deepEqual(await Promise.all([first, second]), ["shared", "shared"]);
    assert.equal(runs, 1);
  });
});

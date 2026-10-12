import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  ensureComponentForRemote,
  mapComponentProgressToRemote,
  remoteDownloadRatio,
  type ComponentProgressLike,
  type RemoteProgressEvent,
} from "./remote-progress";

describe("remoteDownloadRatio", () => {
  const cases: Array<{
    name: string;
    range: [number, number];
    received: number | undefined;
    total: number | undefined;
    expected: number;
  }> = [
    { name: "maps 0 received to low bound", range: [0.1, 0.3], received: 0, total: 100, expected: 0.1 },
    { name: "maps midpoint into range", range: [0.1, 0.3], received: 50, total: 100, expected: 0.2 },
    { name: "maps full received to high bound", range: [0.1, 0.3], received: 100, total: 100, expected: 0.3 },
    { name: "clamps overshoot to high bound", range: [0.06, 0.12], received: 200, total: 100, expected: 0.12 },
    { name: "missing total uses low bound", range: [0.06, 0.12], received: 50, total: undefined, expected: 0.06 },
    { name: "zero total uses low bound", range: [0.06, 0.12], received: 50, total: 0, expected: 0.06 },
    { name: "missing received uses low bound", range: [0.14, 0.38], received: undefined, total: 100, expected: 0.14 },
  ];

  for (const c of cases) {
    it(c.name, () => {
      assert.equal(remoteDownloadRatio(c.range, c.received, c.total), c.expected);
    });
  }
});

describe("mapComponentProgressToRemote", () => {
  it("emits download stage with mapped ratio for matching non-error events", () => {
    const event: ComponentProgressLike = {
      id: "embed-model",
      phase: "downloading",
      received: 25,
      total: 100,
      message: "Downloading Embedding model…",
    };
    assert.deepEqual(mapComponentProgressToRemote(event, "embed-model", [0.06, 0.12]), {
      stage: "download",
      ratio: 0.075,
      message: "Downloading Embedding model…",
    });
  });

  it("skips wrong id and error phases so failures stay on the ensure path", () => {
    assert.equal(
      mapComponentProgressToRemote(
        { id: "linux-server", phase: "downloading", received: 1, total: 2, message: "x" },
        "embed-model",
        [0, 1],
      ),
      null,
    );
    assert.equal(
      mapComponentProgressToRemote(
        { id: "embed-model", phase: "error", message: "boom", received: 0, total: 1 },
        "embed-model",
        [0, 1],
      ),
      null,
    );
  });
});

describe("ensureComponentForRemote", () => {
  it("mirrors progress callbacks into the remote bar within the range", async () => {
    const emitted: RemoteProgressEvent[] = [];
    const listeners = new Set<(event: ComponentProgressLike) => void>();
    const result = await ensureComponentForRemote({
      id: "linux-server",
      range: [0.14, 0.38],
      ensure: async (id) => {
        assert.equal(id, "linux-server");
        for (const listener of listeners) {
          listener({
            id: "linux-server",
            phase: "downloading",
            received: 60,
            total: 120,
            message: "Downloading Linux server bundle…",
          });
          listener({
            id: "linux-server",
            phase: "done",
            received: 120,
            total: 120,
            message: "Linux server bundle ready",
          });
        }
        return { id, path: "/tmp/linux.tar.gz", downloaded: true };
      },
      subscribe: (listener) => {
        listeners.add(listener);
      },
      unsubscribe: (listener) => {
        listeners.delete(listener);
      },
      emit: (event) => emitted.push(event),
    });

    assert.deepEqual(result, { id: "linux-server", path: "/tmp/linux.tar.gz", downloaded: true });
    assert.equal(emitted.length, 2);
    assert.equal(emitted[0]!.stage, "download");
    assert.equal(emitted[0]!.ratio, 0.26);
    assert.equal(emitted[1]!.ratio, 0.38);
    assert.equal(listeners.size, 0, "listener removed after ensure settles");
  });

  it("propagates ensure failures and still removes the listener", async () => {
    const emitted: RemoteProgressEvent[] = [];
    const listeners = new Set<(event: ComponentProgressLike) => void>();
    await assert.rejects(
      ensureComponentForRemote({
        id: "embed-model",
        range: [0.06, 0.12],
        ensure: async () => {
          for (const listener of listeners) {
            listener({
              id: "embed-model",
              phase: "error",
              message: "Embedding model download failed",
              received: 0,
              total: 1,
            });
          }
          throw new Error("Embedding model download failed: sha256 mismatch");
        },
        subscribe: (listener) => {
          listeners.add(listener);
        },
        unsubscribe: (listener) => {
          listeners.delete(listener);
        },
        emit: (event) => emitted.push(event),
      }),
      /sha256 mismatch/,
    );
    assert.equal(emitted.length, 0, "error phase is not mirrored into the remote bar");
    assert.equal(listeners.size, 0, "listener removed even on failure");
  });
});

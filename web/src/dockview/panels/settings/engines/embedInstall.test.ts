import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { ComponentProgressEvent } from "../../../../api/auth";
import type { EnginesDetail } from "../../../../api/workspace";
import * as workspace from "../../../../api/workspace";
import {
  ENGINES_CHANGED_EVENT,
  installEmbedInstallWatcher,
  restartRetrievalAfterEmbedInstall,
} from "./embedInstall";
import { stubDesktopBridge } from "../../../../test/desktopBridge";

const getEnginesDetail = vi.spyOn(workspace, "getEnginesDetail");
const refreshRetrieval = vi.spyOn(workspace, "refreshRetrieval");

function detail(usable: EnginesDetail["retrieval"]["usable"]): EnginesDetail {
  return { retrieval: { usable } } as unknown as EnginesDetail;
}

let changed = 0;
const onChanged = () => {
  changed += 1;
};

beforeEach(() => {
  changed = 0;
  getEnginesDetail.mockReset();
  refreshRetrieval.mockReset().mockResolvedValue({ desired: true, mode: "full" } as never);
  window.addEventListener(ENGINES_CHANGED_EVENT, onChanged);
});

afterEach(() => {
  window.removeEventListener(ENGINES_CHANGED_EVENT, onChanged);
  delete window.litecode;
});

describe("restartRetrievalAfterEmbedInstall", () => {
  it("restarts a retrieval engine that failed for lack of the model, then notifies views", async () => {
    getEnginesDetail.mockResolvedValue(detail("unavailable"));
    await restartRetrievalAfterEmbedInstall();
    expect(refreshRetrieval).toHaveBeenCalledTimes(1);
    expect(changed).toBe(1);
  });

  it.each(["stopped", "ready"] as const)("leaves a %s engine alone but still notifies", async (usable) => {
    getEnginesDetail.mockResolvedValue(detail(usable));
    await restartRetrievalAfterEmbedInstall();
    expect(refreshRetrieval).not.toHaveBeenCalled();
    expect(changed).toBe(1);
  });

  it("waits out a warmup that started before the model landed", async () => {
    getEnginesDetail
      .mockResolvedValueOnce(detail("warming"))
      .mockResolvedValueOnce(detail("warming"))
      .mockResolvedValue(detail("unavailable"));
    await restartRetrievalAfterEmbedInstall({ pollMs: 1 });
    expect(getEnginesDetail).toHaveBeenCalledTimes(3);
    expect(refreshRetrieval).toHaveBeenCalledTimes(1);
  });

  it("dedupes overlapping calls (card + app watcher) and survives server errors", async () => {
    getEnginesDetail.mockResolvedValue(detail("unavailable"));
    refreshRetrieval.mockRejectedValueOnce(new Error("engine not desired"));
    const warn = vi.spyOn(console, "warn").mockImplementation(() => undefined);
    await Promise.all([restartRetrievalAfterEmbedInstall(), restartRetrievalAfterEmbedInstall()]);
    expect(refreshRetrieval).toHaveBeenCalledTimes(1);
    expect(changed).toBe(1);
    warn.mockRestore();
  });
});

describe("installEmbedInstallWatcher", () => {
  function bridge(mode: "local" | "remote") {
    let handler: ((event: ComponentProgressEvent) => void) | null = null;
    window.litecode = stubDesktopBridge({
      getSessionMode: () => mode,
      onComponentProgress: (h) => {
        handler = h;
        return () => {
          handler = null;
        };
      },
    });
    return { emit: (event: ComponentProgressEvent) => handler?.(event), attached: () => handler !== null };
  }

  it("restarts retrieval on embed-model done only", async () => {
    getEnginesDetail.mockResolvedValue(detail("unavailable"));
    const host = bridge("local");
    const stop = installEmbedInstallWatcher();
    host.emit({ id: "embed-model", phase: "downloading", percent: 0, message: "…" });
    host.emit({ id: "linux-server", phase: "done", percent: 0, message: "ready" });
    expect(getEnginesDetail).not.toHaveBeenCalled();
    host.emit({ id: "embed-model", phase: "done", percent: 0, message: "ready" });
    await vi.waitFor(() => expect(refreshRetrieval).toHaveBeenCalledTimes(1));
    stop();
    expect(host.attached()).toBe(false);
  });

  it("does nothing in remote sessions or without the desktop bridge", () => {
    expect(() => installEmbedInstallWatcher()()).not.toThrow();
    const host = bridge("remote");
    installEmbedInstallWatcher();
    expect(host.attached()).toBe(false);
  });
});

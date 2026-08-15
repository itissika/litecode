import assert from "node:assert/strict";
import { describe, it } from "node:test";
import type { WebContents } from "electron";

import { senderOwnedWindow, type BrowserWindowLike } from "./window-guard";


type FakeWebContents = Pick<WebContents, "id"> & { _destroyed: boolean };

function webContents(id: number, destroyed = false): FakeWebContents {
  return { id, _destroyed: destroyed };
}

function liveWindow(): BrowserWindowLike & { id: number } {
  return { id: 7, isDestroyed: () => false };
}

function deadWindow(): BrowserWindowLike & { id: number } {
  return { id: 7, isDestroyed: () => true };
}

describe("senderOwnedWindow (DESK-01 targetWindow ownership)", () => {
  it("returns the sender's live window", () => {
    const win = liveWindow();
    const got = senderOwnedWindow(
      webContents(1) as unknown as WebContents,
      () => win,
    );
    assert.equal(got, win);
  });

  it("refuses when the sender's webContents resolves to a destroyed window", () => {
    const win = deadWindow();
    const got = senderOwnedWindow(
      webContents(1) as unknown as WebContents,
      () => win,
    );
    assert.equal(got, null);
  });

  it("refuses when the sender's webContents resolves to no window (no global fallback)", () => {
    const got = senderOwnedWindow(webContents(1) as unknown as WebContents, () => null);
    assert.equal(got, null);
  });

  it("refuses when there is no sender webContents", () => {
    const got = senderOwnedWindow(null, () => liveWindow());
    assert.equal(got, null);
  });
});

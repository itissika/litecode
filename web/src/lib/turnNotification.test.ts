import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { TurnFinished } from "../api/types";
import { stubDesktopBridge } from "../test/desktopBridge";
import {
  maybeNotifyTurnEnd,
  resetTurnNotifications,
  turnNotificationBody,
} from "./turnNotification";

function finished(overrides: Partial<TurnFinished> = {}): TurnFinished {
  return {
    turn_id: "turn-1",
    reason: "completed",
    final_text: "Done.\nSecond line.",
    error: null,
    snapshot: {},
    session_id: "session-1",
    ...overrides,
  } as unknown as TurnFinished;
}

beforeEach(() => {
  resetTurnNotifications();
  vi.spyOn(document, "hasFocus").mockReturnValue(false);
});

afterEach(() => {
  vi.restoreAllMocks();
  delete window.litecode;
});

describe("turnNotificationBody", () => {
  it("uses the first non-empty line of the final text", () => {
    expect(turnNotificationBody(finished())).toBe("Done.");
    expect(turnNotificationBody(finished({ final_text: "\n\n  spaced  out  " }))).toBe(
      "spaced out",
    );
  });

  it("falls back to Completed when the final text is empty", () => {
    expect(turnNotificationBody(finished({ final_text: "  \n " }))).toBe("Completed");
  });

  it("reports the error message", () => {
    expect(
      turnNotificationBody(
        finished({
          reason: "error",
          final_text: null,
          error: { code: "llm", message: "provider exploded" } as never,
        }),
      ),
    ).toBe("provider exploded");
  });

  it("names the step limit and cancellation", () => {
    expect(turnNotificationBody(finished({ reason: "max_steps" }))).toBe(
      "Stopped at the step limit",
    );
    expect(turnNotificationBody(finished({ reason: "cancelled" }))).toBe("Stopped");
  });
});

describe("maybeNotifyTurnEnd", () => {
  it("notifies once per turn while the window is unseen", () => {
    const notifyTurnEnd = vi.fn();
    window.litecode = stubDesktopBridge({ notifyTurnEnd });
    maybeNotifyTurnEnd(finished(), "Fix the build");
    expect(notifyTurnEnd).toHaveBeenCalledTimes(1);
    expect(notifyTurnEnd).toHaveBeenCalledWith({
      sessionId: "session-1",
      turnId: "turn-1",
      title: "Fix the build",
      body: "Done.",
    });
    maybeNotifyTurnEnd(finished(), "Fix the build");
    expect(notifyTurnEnd).toHaveBeenCalledTimes(1);
  });

  it("stays silent while the window has focus", () => {
    vi.spyOn(document, "hasFocus").mockReturnValue(true);
    const notifyTurnEnd = vi.fn();
    window.litecode = stubDesktopBridge({ notifyTurnEnd });
    maybeNotifyTurnEnd(finished(), "");
    expect(notifyTurnEnd).not.toHaveBeenCalled();
  });

  it("does nothing without the desktop bridge", () => {
    expect(() => maybeNotifyTurnEnd(finished(), "")).not.toThrow();
  });
});

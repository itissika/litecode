import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import i18n from "../i18n";
import { stubDesktopBridge } from "../test/desktopBridge";
import { WindowCloseGuard } from "./WindowCloseGuard";

let ask: (() => void) | null = null;
const answerCloseRequest = vi.fn(async () => {});

beforeEach(() => {
  ask = null;
  answerCloseRequest.mockClear();
  window.litecode = stubDesktopBridge({
    onCloseRequest: (handler) => {
      ask = handler;
      return () => {
        ask = null;
      };
    },
    answerCloseRequest,
  });
});

afterEach(() => {
  cleanup();
  delete window.litecode;
});

describe("WindowCloseGuard", () => {
  it("acknowledges as soon as the question is up, then answers yes", async () => {
    render(<WindowCloseGuard />);
    expect(ask).toBeTruthy();

    act(() => ask?.());
    const dialog = await screen.findByRole("dialog");
    expect(dialog.textContent).toContain(i18n.t("chrome.closeRunning.title"));
    // `shown` before any human input: it stops the host's fallback prompt.
    expect(answerCloseRequest).toHaveBeenCalledWith("shown");

    fireEvent.click(
      screen.getByRole("button", { name: i18n.t("chrome.closeRunning.confirm") }),
    );
    await waitFor(() => expect(answerCloseRequest).toHaveBeenLastCalledWith("close"));
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("answers cancel when the person keeps working", async () => {
    render(<WindowCloseGuard />);

    act(() => ask?.());
    await screen.findByRole("dialog");
    fireEvent.click(
      screen.getByRole("button", { name: i18n.t("chrome.closeRunning.cancel") }),
    );

    await waitFor(() => expect(answerCloseRequest).toHaveBeenLastCalledWith("cancel"));
    expect(answerCloseRequest).toHaveBeenCalledTimes(2);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("does nothing in browser mode, where there is no host", () => {
    delete window.litecode;
    expect(() => render(<WindowCloseGuard />)).not.toThrow();
    expect(screen.queryByRole("dialog")).toBeNull();
  });
});

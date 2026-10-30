import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { useToastStore } from "../stores/toastStore";
import { ToastHost } from "./ToastHost";

afterEach(() => {
  cleanup();
  useToastStore.setState({ toasts: [], paused: false });
});

describe("ToastHost rich toasts", () => {
  it("renders the emoji icon instead of the variant dot when one is set", () => {
    useToastStore.getState().showToast("plain note", "info");
    useToastStore
      .getState()
      .showToast("rich note", "info", 5000, "rich", { icon: "🎉" });

    const { container } = render(<ToastHost />);

    expect(screen.getByText("🎉")).toBeTruthy();
    // Exactly one dot: the plain toast. The icon toast renders the emoji instead.
    const dots = container.querySelectorAll("span.rounded-full");
    expect(dots).toHaveLength(1);
  });

  it("renders markdown instead of showing the raw source", () => {
    useToastStore
      .getState()
      .showToast("🫠 **还没配置 key**，agent 用不了。", "info");

    const { container } = render(<ToastHost />);

    expect(container.querySelector("strong")?.textContent).toBe(
      "还没配置 key",
    );
    expect(container.textContent).not.toContain("**");
  });
});

describe("ToastHost errors", () => {
  it("keeps the error raw inside its own list", () => {
    const raw = "Request failed: 400\n\nbody: **quota exceeded**";
    useToastStore.getState().showToast(raw, "error");

    const { container } = render(<ToastHost />);

    const list = screen.getByRole("group", { name: "Error details" });
    expect(list.textContent).toBe(raw);
    expect(container.querySelector("strong")).toBeNull();
  });

  it("copies the untouched message from the list", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", {
      value: { writeText },
      configurable: true,
    });
    const raw = "engine crashed\n  at turn 3";
    useToastStore.getState().showToast(raw, "error");

    render(<ToastHost />);
    fireEvent.click(screen.getByRole("button", { name: "Copy error message" }));

    await waitFor(() => expect(writeText).toHaveBeenCalledWith(raw));
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Copied" })).toBeTruthy(),
    );
  });
});

describe("ToastHost folding and hover", () => {
  it("shows how many identical notes folded into one row", () => {
    useToastStore.getState().showToast("same", "success");
    useToastStore.getState().showToast("same", "success");

    render(<ToastHost />);

    expect(screen.getByLabelText("2 identical notes").textContent).toBe("2");
  });

  it("holds the stack while hovered and releases it on leave", () => {
    useToastStore.getState().showToast("note", "info");
    const { container } = render(<ToastHost />);
    const stack = container.querySelector('[aria-live="polite"]');
    expect(stack).toBeTruthy();

    fireEvent.mouseOver(stack as Element);
    expect(useToastStore.getState().paused).toBe(true);

    fireEvent.mouseOut(stack as Element);
    expect(useToastStore.getState().paused).toBe(false);
  });
});

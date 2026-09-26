import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { useToastStore } from "../stores/toastStore";
import { ToastHost } from "./ToastHost";

afterEach(() => {
  cleanup();
  useToastStore.setState({ toasts: [] });
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

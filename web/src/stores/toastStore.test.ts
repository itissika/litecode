import { afterEach, describe, expect, it, vi } from "vitest";

import { useToastStore } from "./toastStore";

afterEach(() => {
  useToastStore.setState({ toasts: [] });
  vi.useRealTimers();
});

describe("toast channel upsert", () => {
  it("replaces an existing toast with the same channel instead of stacking", () => {
    vi.useFakeTimers();
    useToastStore.getState().showToast("first", "error", 5000, "settings-persist-error");
    useToastStore.getState().showToast("second", "error", 5000, "settings-persist-error");
    const toasts = useToastStore.getState().toasts;
    expect(toasts).toHaveLength(1);
    expect(toasts[0].message).toBe("second");
    expect(toasts[0].id).toBe("settings-persist-error");
  });
});

import { afterEach, describe, expect, it, vi } from "vitest";

import { useToastStore } from "./toastStore";

afterEach(() => {
  useToastStore.getState().setPaused(false);
  useToastStore.setState({ toasts: [], paused: false });
  vi.useRealTimers();
});

describe("toast channel upsert", () => {
  it("replaces an existing toast with the same channel instead of stacking", () => {
    vi.useFakeTimers();
    useToastStore
      .getState()
      .showToast("first", "error", 5000, "settings-persist-error");
    useToastStore
      .getState()
      .showToast("second", "error", 5000, "settings-persist-error");
    const toasts = useToastStore.getState().toasts;
    expect(toasts).toHaveLength(1);
    expect(toasts[0].message).toBe("second");
    expect(toasts[0].id).toBe("settings-persist-error");
  });

  it("counts a slot re-raised with the exact same text", () => {
    vi.useFakeTimers();
    useToastStore.getState().showToast("same", "error", 5000, "channel");
    useToastStore.getState().showToast("same", "error", 5000, "channel");

    const toasts = useToastStore.getState().toasts;
    expect(toasts).toHaveLength(1);
    expect(toasts[0].count).toBe(2);
  });
});

describe("identical notes fold together", () => {
  it("keeps one row and counts the repeats", () => {
    vi.useFakeTimers();
    const { showToast } = useToastStore.getState();
    showToast("Request failed", "error", 5000);
    showToast("Request failed", "error", 5000);
    showToast("Request failed", "error", 5000);

    const toasts = useToastStore.getState().toasts;
    expect(toasts).toHaveLength(1);
    expect(toasts[0].count).toBe(3);
  });

  it("does not fold the same text across variants", () => {
    vi.useFakeTimers();
    const { showToast } = useToastStore.getState();
    showToast("copy failed", "error", 5000);
    showToast("copy failed", "info", 5000);

    expect(useToastStore.getState().toasts).toHaveLength(2);
  });

  it("gives the folded row the full reading time again", () => {
    vi.useFakeTimers();
    const { showToast } = useToastStore.getState();
    showToast("Request failed", "error", 5000);
    vi.advanceTimersByTime(4000);
    showToast("Request failed", "error", 5000);

    vi.advanceTimersByTime(4000);
    expect(useToastStore.getState().toasts).toHaveLength(1);

    vi.advanceTimersByTime(1100);
    expect(useToastStore.getState().toasts).toHaveLength(0);
  });
});

describe("hover hold", () => {
  it("holds every timer while paused and resumes with the time left", () => {
    vi.useFakeTimers();
    const { showToast } = useToastStore.getState();
    showToast("first", "error", 5000);
    showToast("second", "info", 5000);
    vi.advanceTimersByTime(4000);

    useToastStore.getState().setPaused(true);
    vi.advanceTimersByTime(60_000);
    expect(useToastStore.getState().toasts).toHaveLength(2);

    useToastStore.getState().setPaused(false);
    vi.advanceTimersByTime(900);
    expect(useToastStore.getState().toasts).toHaveLength(2);

    vi.advanceTimersByTime(200);
    expect(useToastStore.getState().toasts).toHaveLength(0);
  });

  it("holds a note raised while the stack is hovered", () => {
    vi.useFakeTimers();
    useToastStore.getState().setPaused(true);
    useToastStore.getState().showToast("late note", "error", 5000);

    vi.advanceTimersByTime(60_000);
    expect(useToastStore.getState().toasts).toHaveLength(1);

    useToastStore.getState().setPaused(false);
    vi.advanceTimersByTime(5100);
    expect(useToastStore.getState().toasts).toHaveLength(0);
  });

  it("releases the hold once the stack empties", () => {
    vi.useFakeTimers();
    const { showToast, dismissToast, setPaused } = useToastStore.getState();
    showToast("only", "error", 5000);
    setPaused(true);
    dismissToast(useToastStore.getState().toasts[0].id);

    expect(useToastStore.getState().paused).toBe(false);
  });
});

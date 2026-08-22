import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { useBashStore } from "../stores/bashStore";
import { TerminalStatusBar } from "./TerminalStatusBar";

const job = {
  id: "bg_a",
  call_id: "c1",
  command_preview: "sleep 1",
  output_file: ".litecode/bash/bg_a.output",
  started_at_ms: Date.now(),
};

const twoJobs = [
  job,
  {
    id: "bg_b",
    call_id: "c2",
    command_preview: "sleep 2",
    output_file: ".litecode/bash/bg_b.output",
    started_at_ms: Date.now(),
  },
];

afterEach(() => {
  vi.useRealTimers();
  cleanup();
  useBashStore.getState().reset();
});

describe("TerminalStatusBar", () => {
  it("renders nothing when there are no alive bash jobs", () => {
    const { container } = render(<TerminalStatusBar sessionId="s1" />);
    expect(container.firstChild).toBeNull();
  });

  it("shows a bouncing terminal icon and session job count after the 1s debounce", () => {
    vi.useFakeTimers();
    useBashStore.getState().applySnapshot("s1", { jobs: twoJobs, waits: [] });
    render(<TerminalStatusBar sessionId="s1" />);
    // Not yet — the job must run continuously for 1s before surfacing.
    expect(screen.queryByRole("button", { name: /active terminal/ })).toBeNull();
    act(() => vi.advanceTimersByTime(1000));
    expect(screen.getByLabelText("2 active terminals")).toBeTruthy();
    expect(screen.getByText("×2")).toBeTruthy();
    expect(document.querySelector(".terminal-status-icon")).toBeTruthy();
  });

  it("cycles reveal to each job call_id on click", () => {
    vi.useFakeTimers();
    const onRevealBash = vi.fn();
    useBashStore.getState().applySnapshot("s1", { jobs: twoJobs, waits: [] });
    render(<TerminalStatusBar sessionId="s1" onRevealBash={onRevealBash} />);
    act(() => vi.advanceTimersByTime(1000));
    const chip = screen.getByRole("button", { name: "2 active terminals" });
    fireEvent.click(chip);
    fireEvent.click(chip);
    fireEvent.click(chip);
    expect(onRevealBash.mock.calls.map((c) => c[0])).toEqual(["c1", "c2", "c1"]);
  });

  it("never shows the chip for a job that finishes before the 1s debounce", () => {
    vi.useFakeTimers();
    render(<TerminalStatusBar sessionId="s1" />);

    act(() => {
      useBashStore.getState().applySnapshot("s1", { jobs: [job], waits: [] });
    });
    // Job finishes after 50ms — the debounce never fires, so no chip at all.
    act(() => vi.advanceTimersByTime(50));
    act(() => {
      useBashStore.getState().applySnapshot("s1", { jobs: [], waits: [] });
    });
    act(() => vi.advanceTimersByTime(2000));
    expect(screen.queryByRole("button", { name: /active terminal/ })).toBeNull();
  });

  it("makes the ×0 hold-over chip inert", () => {
    vi.useFakeTimers();
    const onRevealBash = vi.fn();
    render(<TerminalStatusBar sessionId="s1" onRevealBash={onRevealBash} />);

    act(() => {
      useBashStore.getState().applySnapshot("s1", { jobs: [job], waits: [] });
    });
    act(() => vi.advanceTimersByTime(1000));
    act(() => {
      useBashStore.getState().applySnapshot("s1", { jobs: [], waits: [] });
    });

    const chip = screen.getByRole("button", { name: "0 active terminals" });
    expect((chip as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(chip);
    expect(onRevealBash).not.toHaveBeenCalled();
  });

  it("holds the chip 1s even after a long-running job ends", () => {
    vi.useFakeTimers();
    render(<TerminalStatusBar sessionId="s1" />);

    act(() => {
      useBashStore.getState().applySnapshot("s1", { jobs: [job], waits: [] });
    });
    act(() => vi.advanceTimersByTime(1000));
    expect(screen.getByLabelText("1 active terminal")).toBeTruthy();

    // Job runs 10s total, then ends — the hold applies the same as ever.
    act(() => vi.advanceTimersByTime(9000));
    act(() => {
      useBashStore.getState().applySnapshot("s1", { jobs: [], waits: [] });
    });
    expect(screen.getByLabelText("0 active terminals")).toBeTruthy();

    act(() => vi.advanceTimersByTime(500));
    expect(screen.getByLabelText("0 active terminals")).toBeTruthy();

    // Hold expires at t=11000; exit animation runs before unmount.
    act(() => vi.advanceTimersByTime(600));
    act(() => vi.advanceTimersByTime(300));
    expect(screen.queryByRole("button", { name: /active terminal/ })).toBeNull();
  });

  it("animates in on mount and out before unmount", () => {
    vi.useFakeTimers();
    render(<TerminalStatusBar sessionId="s1" />);

    act(() => {
      useBashStore.getState().applySnapshot("s1", { jobs: [job], waits: [] });
    });
    act(() => vi.advanceTimersByTime(1000));
    const chip = screen.getByRole("button", { name: "1 active terminal" });
    // Mounted closed, opens on the next frame so the CSS transition runs.
    expect(chip.className).not.toContain("is-open");
    act(() => vi.advanceTimersByTime(20));
    expect(chip.className).toContain("is-open");

    // Job ends → hold expires → chip retracts (is-open dropped) before unmount.
    act(() => {
      useBashStore.getState().applySnapshot("s1", { jobs: [], waits: [] });
    });
    act(() => vi.advanceTimersByTime(1000));
    expect(chip.className).not.toContain("is-open");

    act(() => vi.advanceTimersByTime(300));
    expect(screen.queryByRole("button", { name: /active terminal/ })).toBeNull();
  });

  it("re-arms the hold when a new job starts during the grace window", () => {
    vi.useFakeTimers();
    render(<TerminalStatusBar sessionId="s1" />);

    // Job A runs 1.2s (surfaces at 1s), then ends at t=1.2s → hold to t=2.2s.
    act(() => {
      useBashStore.getState().applySnapshot("s1", { jobs: [job], waits: [] });
    });
    act(() => vi.advanceTimersByTime(1200));
    act(() => {
      useBashStore.getState().applySnapshot("s1", { jobs: [], waits: [] });
    });

    // Job B starts at t=1.5s (inside A's hold) and ends at t=1.6s.
    act(() => vi.advanceTimersByTime(300));
    act(() => {
      useBashStore.getState().applySnapshot("s1", { jobs: [job], waits: [] });
    });
    act(() => vi.advanceTimersByTime(100));
    act(() => {
      useBashStore.getState().applySnapshot("s1", { jobs: [], waits: [] });
    });

    // B's hold runs from t=1.6s → 2.6s: chip still up at t=2.4s…
    act(() => vi.advanceTimersByTime(800));
    expect(screen.getByLabelText("0 active terminals")).toBeTruthy();

    // …gone after the re-armed deadline + exit animation.
    act(() => vi.advanceTimersByTime(200));
    act(() => vi.advanceTimersByTime(300));
    expect(screen.queryByRole("button", { name: /active terminal/ })).toBeNull();
  });
});

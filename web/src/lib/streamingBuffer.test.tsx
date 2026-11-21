import { act, cleanup, render } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { PanelVisibilityProvider } from "./panelVisibility";
import { useStreamingBuffer } from "./streamingBuffer";

function Probe({ text, streaming }: { text: string; streaming: boolean }) {
  const shown = useStreamingBuffer(text, streaming);
  return <span data-testid="shown">{shown}</span>;
}

function read(): string {
  return document.querySelector('[data-testid="shown"]')!.textContent ?? "";
}

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe("useStreamingBuffer visibility gate", () => {
  it("shows the authoritative text immediately in an inactive panel", () => {
    vi.useFakeTimers();
    const view = render(
      <PanelVisibilityProvider active={false}>
        <Probe text="" streaming />
      </PanelVisibilityProvider>,
    );

    view.rerender(
      <PanelVisibilityProvider active={false}>
        <Probe text="complete assistant text" streaming />
      </PanelVisibilityProvider>,
    );

    expect(read()).toBe("complete assistant text");
    expect(vi.getTimerCount()).toBe(0);
  });

  it("stops a running animation when the panel goes inactive", () => {
    vi.useFakeTimers();
    const view = render(
      <PanelVisibilityProvider active>
        <Probe text="" streaming />
      </PanelVisibilityProvider>,
    );

    view.rerender(
      <PanelVisibilityProvider active>
        <Probe text="abcdefghij" streaming />
      </PanelVisibilityProvider>,
    );
    act(() => vi.advanceTimersByTime(28));
    expect(read().length).toBeLessThan("abcdefghij".length);

    view.rerender(
      <PanelVisibilityProvider active={false}>
        <Probe text="abcdefghij" streaming />
      </PanelVisibilityProvider>,
    );

    expect(read()).toBe("abcdefghij");
    expect(vi.getTimerCount()).toBe(0);
  });

  it("animates again once the panel is active", () => {
    vi.useFakeTimers();
    const view = render(
      <PanelVisibilityProvider active={false}>
        <Probe text="" streaming />
      </PanelVisibilityProvider>,
    );

    view.rerender(
      <PanelVisibilityProvider active>
        <Probe text="abcdefghij" streaming />
      </PanelVisibilityProvider>,
    );
    act(() => vi.advanceTimersByTime(28));

    expect(read()).toBe("a");
  });

  it("keeps animating when no provider is mounted", () => {
    vi.useFakeTimers();
    const view = render(<Probe text="" streaming />);

    view.rerender(<Probe text="abc" streaming />);
    act(() => vi.advanceTimersByTime(28));

    expect(read()).toBe("a");
  });
});

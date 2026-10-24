import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import type { ContextMeter } from "../api/types";
import { useConnectionStore } from "../stores/connectionStore";
import { EMPTY_SLICE, useTurnStore } from "../stores/turnStore";
import { ContextUsageRing } from "./ContextUsageRing";

const SESSION_ID = "s-context-meter";

function meter(compactEligible: boolean, usedTokens = 800): ContextMeter {
  return {
    context_window: 1_000,
    output_reserve: 100,
    safety_headroom: 125,
    prompt_limit: 900,
    compact_at: 775,
    used_tokens: usedTokens,
    source: "provider_plus_suffix",
    compact_eligible: compactEligible,
  };
}

function seed(compactEligible: boolean) {
  useTurnStore.setState({
    byId: new Map([
      [
        SESSION_ID,
        {
          ...EMPTY_SLICE,
          contextWindow: 1_000,
          contextTokensEstimate: 900,
          lastTurnPromptTokens: 900,
          contextMeter: meter(compactEligible),
          compactEligible,
          runState: "idle" as const,
        },
      ],
    ]),
  });
}

describe("ContextUsageRing server meter", () => {
  beforeEach(() => {
    useConnectionStore.setState({ state: "connected" } as never);
  });

  afterEach(() => {
    cleanup();
    useTurnStore.setState({ byId: new Map() });
  });

  it("does not enable compact from raw provider/transcript numbers alone", () => {
    seed(false);
    render(<ContextUsageRing sessionId={SESSION_ID} />);
    fireEvent.click(screen.getByTitle("Context usage"));
    const button = screen.getByLabelText("Compact context") as HTMLButtonElement;
    expect(button.disabled).toBe(true);
  });

  it("replaces a stale snapshot meter from live request/completion events", () => {
    seed(false);
    useTurnStore.setState((state) => {
      const byId = new Map(state.byId);
      const slice = byId.get(SESSION_ID)!;
      byId.set(SESSION_ID, {
        ...slice,
        currentTurnId: "t-live",
        runState: "running" as const,
        contextMeter: meter(false, 100),
        contextTokensEstimate: 100,
      });
      return { byId };
    });
    render(<ContextUsageRing sessionId={SESSION_ID} />);
    fireEvent.click(screen.getByTitle("Context usage"));
    expect(screen.getByTitle("Context usage").textContent).toContain("10");
    const button = screen.getByLabelText("Compact context") as HTMLButtonElement;
    expect(button.disabled).toBe(true);

    act(() => {
      useTurnStore.getState().onTurnEvent({
        session_id: SESSION_ID,
        turn_id: "t-live",
        event: {
          type: "llm_request_built",
          model: "m",
          endpoint: "responses",
          token_estimate: 100,
          tools_count: 2,
          context_window: 1_000,
          context_meter: meter(true, 900),
        },
      });
    });

    expect(screen.getByTitle("Context usage").textContent).toContain("90");
    expect(useTurnStore.getState().byId.get(SESSION_ID)?.contextMeter?.used_tokens).toBe(900);
    act(() => {
      useTurnStore.setState((state) => {
        const byId = new Map(state.byId);
        const slice = byId.get(SESSION_ID)!;
        byId.set(SESSION_ID, { ...slice, runState: "idle" as const });
        return { byId };
      });
    });
    expect(button.disabled).toBe(false);
  });

  it("enables compact when the server meter decision is true", () => {
    seed(true);
    render(<ContextUsageRing sessionId={SESSION_ID} />);
    fireEvent.click(screen.getByTitle("Context usage"));
    const button = screen.getByLabelText("Compact context") as HTMLButtonElement;
    expect(button.disabled).toBe(false);

    act(() => {
      useTurnStore.setState((state) => {
        const byId = new Map(state.byId);
        const slice = byId.get(SESSION_ID)!;
        byId.set(SESSION_ID, { ...slice, compactEligible: false });
        return { byId };
      });
    });
    expect(button.disabled).toBe(true);
  });
});

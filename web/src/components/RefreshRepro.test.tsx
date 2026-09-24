import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { useConnectionStore } from "../stores/connectionStore";
import { useMessageStore } from "../stores/messageStore";
import { useSessionStore } from "../stores/sessionStore";
import { useTurnStore } from "../stores/turnStore";
import { AgentChatInput } from "./AgentChatInput";

function seedSession(sessionId: string) {
  useSessionStore.setState({
    primaryAgents: [],
    availableModels: [
      {
        id: "openai/model-1",
        api_model_id: "model-1",
        provider_id: "openai",
        label: "Model 1",
        context_window: 1000,
      },
    ],
    byId: new Map([
      [
        sessionId,
        {
          activePrimary: "default",
          pendingPrimaryId: null,
          modelId: "openai/model-1",
          apiModelId: "model-1",
          label: "Model 1",
          thinkingTier: "medium",
          contextMode: "standard",
          pendingThinkingTier: null,
          pendingContextMode: null,
          maxFileRevertK: null,
        },
      ],
    ]),
  } as never);
}

const RUNNING_TURN = {
  turn_id: "t-1",
  phase: "streaming",
  step: 2,
  step_max: 20,
  started_at_ms: 1,
  awaiting_permission: false,
};

beforeEach(() => {
  useTurnStore.setState({ byId: new Map() });
  useMessageStore.setState({ bySession: new Map() });
  useSessionStore.setState({
    byId: new Map(),
    primaryAgents: [],
    availableModels: [],
    sessions: [],
  } as never);
  useConnectionStore.setState({
    state: "connected",
    sendRpc: vi.fn(async () => ({})),
  } as never);
  seedSession("session-1");
});

afterEach(() => {
  cleanup();
  useTurnStore.setState({ byId: new Map() });
  useMessageStore.setState({ bySession: new Map() });
  useSessionStore.setState({
    byId: new Map(),
    primaryAgents: [],
    availableModels: [],
  } as never);
});

describe("refresh hydration repro", () => {
  it("snapshot with running turn disables the send button", () => {
    // Simulate the post-refresh subscribe flow: session/attached then
    // session/snapshot, both carrying the running turn.
    const conn = useConnectionStore.getState();
    conn.dispatchEnvelope({
      method: "session/attached",
      params: { session_id: "session-1", turn: RUNNING_TURN },
    } as never);
    conn.dispatchEnvelope({
      method: "session/snapshot",
      params: {
        session_id: "session-1",
        project: "p",
        agent_id: "default",
        api_model_id: "model-1",
        model_id: "openai/model-1",
        buffer: { last_seq: 0, next_seq: 10 },
        turn: RUNNING_TURN,
        context_window: 1000,
        context_tokens_estimate: 0,
        compact_eligible: false,
        compacting: false,
        thinking_tier: "medium",
        context_mode: "standard",
      },
    } as never);

    expect(
      useTurnStore.getState().byId.get("session-1")?.runState ?? "idle",
    ).toBe("running");

    render(<AgentChatInput sessionId="session-1" />);
    const ta = screen.getByPlaceholderText("Message the agent...");
    expect(ta).toBeTruthy();
    // The cancel button should be shown while running.
    const cancel = screen.queryByTitle("Cancel");
    const send = screen.queryByTitle("Send");
    expect(cancel).toBeTruthy();
    expect(send).toBeNull();
  });

  it("attached-only (no snapshot) leaves the button sendable — repro", () => {
    const conn = useConnectionStore.getState();
    conn.dispatchEnvelope({
      method: "session/attached",
      params: { session_id: "session-1", turn: RUNNING_TURN },
    } as never);

    expect(
      useTurnStore.getState().byId.get("session-1")?.runState ?? "idle",
    ).toBe("idle");

    render(<AgentChatInput sessionId="session-1" />);
    const send = screen.queryByTitle("Send");
    const cancel = screen.queryByTitle("Cancel");
    expect(cancel).toBeNull();
    expect(send).toBeTruthy();
  });
});

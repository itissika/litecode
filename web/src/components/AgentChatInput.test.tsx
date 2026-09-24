import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { useConnectionStore } from "../stores/connectionStore";
import { useMessageStore } from "../stores/messageStore";
import { useSessionStore } from "../stores/sessionStore";
import { useToastStore } from "../stores/toastStore";
import { EMPTY_SLICE, useTurnStore } from "../stores/turnStore";
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

beforeEach(() => {
  useTurnStore.setState({ byId: new Map() });
  useMessageStore.setState({ bySession: new Map() });
  useToastStore.setState({ toasts: [] });
  useConnectionStore.setState({
    state: "connected",
    sendRpc: vi.fn(async () => ({ started: true })),
  } as never);
  seedSession("session-1");
});

afterEach(() => {
  cleanup();
  useTurnStore.setState({ byId: new Map() });
  useMessageStore.setState({ bySession: new Map() });
  useToastStore.setState({ toasts: [] });
  useSessionStore.setState({
    byId: new Map(),
    primaryAgents: [],
    availableModels: [],
  } as never);
});

describe("AgentChatInput silent send", () => {
  it("does not toast when Enter is blocked by an in-flight optimistic turn", () => {
    useTurnStore.setState({
      byId: new Map([["session-1", { ...EMPTY_SLICE, runState: "running" }]]),
    });
    render(<AgentChatInput sessionId="session-1" />);

    const textarea = screen.getByPlaceholderText("Message the agent...");
    fireEvent.change(textarea, { target: { value: "second try" } });
    fireEvent.keyDown(textarea, { key: "Enter", shiftKey: false });

    expect(useToastStore.getState().toasts).toEqual([]);
    expect(
      useMessageStore.getState().bySession.get("session-1")?.pendingUser,
    ).toBeFalsy();
    expect((textarea as HTMLTextAreaElement).value).toBe("second try");
  });

  it("keeps the draft and does not toast when start rejects", () => {
    useConnectionStore.setState({
      state: "connected",
      sendRpc: undefined,
    } as never);
    render(<AgentChatInput sessionId="session-1" />);

    const textarea = screen.getByPlaceholderText("Message the agent...");
    fireEvent.change(textarea, { target: { value: "hello" } });
    fireEvent.keyDown(textarea, { key: "Enter", shiftKey: false });

    expect(useToastStore.getState().toasts).toEqual([]);
    expect((textarea as HTMLTextAreaElement).value).toBe("hello");
    expect(
      useTurnStore.getState().byId.get("session-1")?.runState ?? "idle",
    ).toBe("idle");
  });
});

describe("AgentChatInput — subagent variant", () => {
  it("keeps model / thinking tier / context mode + the usage ring", () => {
    render(<AgentChatInput sessionId="session-1" variant="subagent" />);

    expect(screen.getByTestId("subagent-controls")).toBeTruthy();
    expect(screen.getByTitle("Model: Model 1")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Med" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Default" })).toBeTruthy();
    expect(screen.getByTitle("Context usage")).toBeTruthy();
  });

  it("mounts no composer: no textarea, no send/cancel, no notification bell", () => {
    render(<AgentChatInput sessionId="session-1" variant="subagent" />);

    expect(document.querySelector("textarea")).toBeNull();
    expect(screen.queryByPlaceholderText("Message the agent...")).toBeNull();
    expect(screen.queryByTitle("Send")).toBeNull();
    expect(screen.queryByTitle("Cancel")).toBeNull();
  });

  it("drops the agent picker: a child's identity is fixed by its profile", () => {
    useSessionStore.setState({
      primaryAgents: [{ id: "default", description: "" }],
    } as never);

    const primary = render(<AgentChatInput sessionId="session-1" />);
    expect(screen.getByTitle("default")).toBeTruthy();
    primary.unmount();

    render(<AgentChatInput sessionId="session-1" variant="subagent" />);
    expect(screen.queryByTitle("default")).toBeNull();
  });

  it("keeps the ring but drops its Compaction action (read-only)", () => {
    const primary = render(<AgentChatInput sessionId="session-1" />);
    fireEvent.click(screen.getByTitle("Context usage"));
    expect(screen.getByLabelText("Compact context")).toBeTruthy();
    primary.unmount();

    render(<AgentChatInput sessionId="session-1" variant="subagent" />);
    fireEvent.click(screen.getByTitle("Context usage"));
    expect(screen.getByText("No context usage yet")).toBeTruthy();
    expect(screen.queryByLabelText("Compact context")).toBeNull();
  });
});

import { cleanup, act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { useConnectionStore } from "../stores/connectionStore";
import { appendComposerText } from "../stores/composerDraft";
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
  it("queues on Enter while a turn runs: no toast, no optimistic row, draft clears on ack", async () => {
    const sendRpc = vi.fn(async (method: string) => {
      if (method === "session/pending-enqueue") {
        return {
          queued: true,
          pending_messages: [{ id: "p1", text: "second try" }],
        };
      }
      return { started: true };
    });
    useConnectionStore.setState({ state: "connected", sendRpc } as never);
    useTurnStore.setState({
      byId: new Map([["session-1", { ...EMPTY_SLICE, runState: "running" }]]),
    });
    render(<AgentChatInput sessionId="session-1" />);

    const textarea = screen.getByPlaceholderText("Message the agent...");
    fireEvent.change(textarea, { target: { value: "second try" } });
    fireEvent.keyDown(textarea, { key: "Enter", shiftKey: false });

    expect(sendRpc).toHaveBeenCalledWith("session/pending-enqueue", {
      text: "second try",
      session_id: "session-1",
    });
    // The queue lives on the server: no optimistic transcript row, no toast.
    expect(
      useMessageStore.getState().bySession.get("session-1")?.pendingUser,
    ).toBeFalsy();
    expect(useToastStore.getState().toasts).toEqual([]);
    await waitFor(() =>
      expect((textarea as HTMLTextAreaElement).value).toBe(""),
    );
    expect(
      useTurnStore.getState().byId.get("session-1")?.pendingMessages,
    ).toEqual([{ id: "p1", text: "second try" }]);
  });

  it("swaps the single live-turn button to queue when a draft exists", () => {
    useTurnStore.setState({
      byId: new Map([["session-1", { ...EMPTY_SLICE, runState: "running" }]]),
    });
    render(<AgentChatInput sessionId="session-1" />);

    // No draft: the one action button is Cancel, glowing with the turn.
    const cancel = screen.getByTitle("Cancel");
    expect(cancel.className).toContain("send-spin-glow");
    expect(screen.queryByTitle("Queue for the next step")).toBeNull();
    expect(screen.queryByTitle("Send")).toBeNull();

    // A draft turns the same slot into the queue action, glow kept.
    const textarea = screen.getByPlaceholderText("Message the agent...");
    fireEvent.change(textarea, { target: { value: "steer" } });
    const queue = screen.getByTitle("Queue for the next step");
    expect(queue.className).toContain("send-spin-glow");
    expect(screen.queryByTitle("Cancel")).toBeNull();
    expect(
      screen.getAllByTitle(/^(Cancel|Queue for the next step|Send)$/),
    ).toHaveLength(1);
  });

  it("appends a recalled queued batch to the draft instead of replacing it", () => {
    render(<AgentChatInput sessionId="session-1" />);
    const textarea = screen.getByPlaceholderText(
      "Message the agent...",
    ) as HTMLTextAreaElement;
    fireEvent.change(textarea, { target: { value: "my draft" } });

    act(() => {
      appendComposerText("session-1", "queued one\n\nqueued two");
    });
    expect(textarea.value).toBe("my draft\n\nqueued one\n\nqueued two");

    // Another session's recall never lands in this composer.
    act(() => {
      appendComposerText("session-2", "elsewhere");
    });
    expect(textarea.value).toBe("my draft\n\nqueued one\n\nqueued two");
  });

  it("fills an empty draft with the recalled text verbatim", () => {
    render(<AgentChatInput sessionId="session-1" />);
    const textarea = screen.getByPlaceholderText(
      "Message the agent...",
    ) as HTMLTextAreaElement;

    act(() => {
      appendComposerText("session-1", "queued");
    });
    expect(textarea.value).toBe("queued");
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

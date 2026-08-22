import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { useSessionStore } from "../stores/sessionStore";
import { MiniChatInput } from "./MiniChatInput";

afterEach(() => {
  cleanup();
  useSessionStore.setState({
    byId: new Map(),
    primaryAgents: [],
    availableModels: [],
  } as never);
});

describe("MiniChatInput", () => {
  it("keeps replay controls while omitting composer-only status controls", () => {
    useSessionStore.setState({
      primaryAgents: [{ id: "default" }],
      availableModels: [
        {
          id: "model-1",
          api_model_id: "model-1",
          label: "Model 1",
          context_window: 1000,
          adapter_id: "openai",
        },
      ],
      byId: new Map([
        [
          "session-1",
          {
            activePrimary: "default",
            pendingPrimaryId: null,
            modelId: "model-1",
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
    const onDismiss = vi.fn();
    const onChange = vi.fn();

    const { rerender } = render(
      <MiniChatInput
        sessionId="session-1"
        draft="original message"
        settings={{
          primaryId: "default",
          modelId: "model-1",
          thinkingTier: "medium",
          contextMode: "standard",
        }}
        onDismiss={onDismiss}
        onChange={onChange}
        onSubmit={vi.fn()}
      />,
    );

    expect(screen.getByTestId("mini-chat-input").hasAttribute("data-mini-chat-input")).toBe(true);
    expect(screen.getByDisplayValue("original message")).toBeTruthy();
    expect(screen.queryByLabelText(/notification/i)).toBeNull();
    expect(screen.queryByLabelText(/context usage/i)).toBeNull();

    fireEvent.change(screen.getByDisplayValue("original message"), {
      target: { value: "edited message" },
    });
    expect(onChange).toHaveBeenCalledWith("edited message", expect.any(Object));

    rerender(
      <MiniChatInput
        sessionId="session-1"
        draft="edited message"
        settings={{
          primaryId: "default",
          modelId: "model-1",
          thinkingTier: "medium",
          contextMode: "standard",
        }}
        onDismiss={onDismiss}
        onChange={onChange}
        onSubmit={vi.fn()}
      />,
    );
    expect(screen.getByDisplayValue("edited message")).toBeTruthy();

    fireEvent.keyDown(screen.getByDisplayValue("edited message"), {
      key: "Escape",
    });
    expect(onDismiss).toHaveBeenCalledOnce();
  });

  it("grows to its content height and caps long drafts", () => {
    const { rerender } = render(
      <MiniChatInput
        sessionId="session-1"
        draft="a long draft"
        settings={{
          primaryId: "default",
          modelId: "model-1",
          thinkingTier: "medium",
          contextMode: "standard",
        }}
        onDismiss={vi.fn()}
        onChange={vi.fn()}
        onSubmit={vi.fn()}
      />,
    );
    const textarea = screen.getByDisplayValue("a long draft") as HTMLTextAreaElement;
    Object.defineProperty(textarea, "scrollHeight", { configurable: true, value: 180 });

    rerender(
      <MiniChatInput
        sessionId="session-1"
        draft="a longer draft"
        settings={{
          primaryId: "default",
          modelId: "model-1",
          thinkingTier: "medium",
          contextMode: "standard",
        }}
        onDismiss={vi.fn()}
        onChange={vi.fn()}
        onSubmit={vi.fn()}
      />,
    );
    expect(textarea.style.height).toBe("180px");

    Object.defineProperty(textarea, "scrollHeight", { configurable: true, value: 400 });
    fireEvent.change(textarea, { target: { value: "very long draft" } });
    expect(textarea.style.height).toBe("256px");
  });
});

import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { symbolMentionSource } from "../lib/knowledge/markers";
import { useSessionStore } from "../stores/sessionStore";
import { MiniChatInput } from "./MiniChatInput";
import { composerText, pressComposerKey, setComposerText } from "./mention/composerDom";

const REPLAY = "Edit and resend";

afterEach(() => {
  cleanup();
  useSessionStore.setState({
    byId: new Map(),
    primaryAgents: [],
    availableModels: [],
  } as never);
});

describe("MiniChatInput", () => {
  it("shows a revert-and-resend hint when the draft is empty", () => {
    render(
      <MiniChatInput
        sessionId="session-1"
        draft=""
        settings={{
          primaryId: "default",
          modelId: "openai/model-1",
          thinkingTier: "medium",
          contextMode: "standard",
        }}
        onDismiss={vi.fn()}
        onChange={vi.fn()}
        onSubmit={vi.fn()}
      />,
    );
    expect(
      document.querySelector(`[aria-label="${REPLAY}"] p`)?.getAttribute("data-placeholder"),
    ).toBe("Revert and resend…");
  });

  it("keeps replay controls while omitting composer-only status controls", () => {
    useSessionStore.setState({
      primaryAgents: [{ id: "default" }],
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
          "session-1",
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
            maxFileRevertSeq: null,
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
          modelId: "openai/model-1",
          thinkingTier: "medium",
          contextMode: "standard",
        }}
        onDismiss={onDismiss}
        onChange={onChange}
        onSubmit={vi.fn()}
      />,
    );

    expect(
      screen
        .getByTestId("mini-chat-input")
        .hasAttribute("data-mini-chat-input"),
    ).toBe(true);
    expect(composerText(REPLAY)).toBe("original message");
    expect(screen.queryByLabelText(/notification/i)).toBeNull();
    expect(screen.queryByLabelText(/context usage/i)).toBeNull();

    setComposerText(REPLAY, "edited message");
    expect(onChange).toHaveBeenCalledWith("edited message", expect.any(Object));

    rerender(
      <MiniChatInput
        sessionId="session-1"
        draft="edited message"
        settings={{
          primaryId: "default",
          modelId: "openai/model-1",
          thinkingTier: "medium",
          contextMode: "standard",
        }}
        onDismiss={onDismiss}
        onChange={onChange}
        onSubmit={vi.fn()}
      />,
    );
    expect(composerText(REPLAY)).toBe("edited message");

    pressComposerKey(REPLAY, "Escape");
    expect(onDismiss).toHaveBeenCalledOnce();
  });

  it("restores a symbol capsule and submits on Enter", async () => {
    const onSubmit = vi.fn();
    const source = symbolMentionSource("src/a.rs", {
      symbol: "fn save",
      lines: "4-9",
      label: "fn save",
    });
    render(
      <MiniChatInput
        sessionId="session-1"
        draft={source}
        settings={{
          primaryId: "default",
          modelId: "openai/model-1",
          thinkingTier: "medium",
          contextMode: "standard",
        }}
        onDismiss={vi.fn()}
        onChange={vi.fn()}
        onSubmit={onSubmit}
      />,
    );
    const chip = await screen.findByRole("button", { name: "a.rs : fn save" });
    expect(chip.closest(".knowledge-token")?.classList.contains("is-symbol")).toBe(true);
    pressComposerKey(REPLAY, "Enter", true);
    expect(onSubmit).not.toHaveBeenCalled();
    pressComposerKey(REPLAY, "Enter");
    expect(onSubmit).toHaveBeenCalledWith(source, expect.any(Object));
  });
});

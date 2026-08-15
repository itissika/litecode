import React from "react";
import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { MessageList } from "./MessageList";
import type { ChatRow } from "../api/adapter";

const grantPermission = vi.fn();

vi.mock("@tanstack/react-virtual", () => ({
  useVirtualizer: ({
    count,
    getItemKey,
  }: {
    count: number;
    getItemKey?: (index: number) => string | number;
  }) => ({
    getVirtualItems: () =>
      Array.from({ length: count }, (_, index) => ({
        key: getItemKey?.(index) ?? index,
        index,
        start: index * 200,
        size: 200,
        end: (index + 1) * 200,
      })),
    getTotalSize: () => count * 200,
    measureElement: () => {},
    scrollToEnd: () => {},
    isAtEnd: () => true,
    options: { scrollMargin: 0 },
  }),
}));

vi.mock("../stores/turnStore", () => ({
  useTurnStore: (selector: (state: Record<string, unknown>) => unknown) =>
    selector({
      byId: new Map([
        ["session-1", {
          pendingPermission: {
            turn_id: "turn-1",
            request_id: "req-abcdef12",
            tool: "bash",
            rule_id: "default",
            summary: "Run bash command",
          },
        }],
      ]),
      grantPermission,
    }),
  emptySlice: () => ({
    runState: "idle",
    currentTurnId: null,
    pendingCancel: false,
    pendingPermission: null,
    turnPhase: null,
    turnStep: null,
    turnStepMax: null,
    contextWindow: 0,
    lastTurnPromptTokens: 0,
    lastTurnCompletionTokens: 0,
    lastTurnCacheHitTokens: 0,
    lastTurnCacheMissTokens: 0,
    sessionPromptTokens: 0,
    sessionCompletionTokens: 0,
    sessionCacheHitTokens: 0,
    sessionCacheMissTokens: 0,
    stopReason: null,
    todoPending: 0,
    todoInProgress: 0,
    todoCompleted: 0,
    todoItems: [],
  }),
}));

vi.mock("../stores/editorStore", () => ({
  useEditorStore: (selector: (state: Record<string, unknown>) => unknown) =>
    selector({ openFile: vi.fn() }),
}));

vi.mock("../stores/sessionStore", () => ({
  useSessionStore: (selector: (state: Record<string, unknown>) => unknown) =>
    selector({ project: "/test/project" }),
}));

const makeScrollRef = () => React.createRef<HTMLDivElement>();

const oneMessage: ChatRow = {
  id: "item-session-1-0",
  item: {
    type: "message",
    role: "assistant",
    id: "msg_1",
    status: "completed",
    content: [{ type: "output_text", text: "hello", annotations: [] }],
  },
};

const liveReasoning: ChatRow = {
  id: "live-rs_1",
  streaming: true,
  item: {
    type: "reasoning",
    id: "rs_1",
    summary: [{ type: "summary_text", text: "thinking" }],
    content: [{ type: "reasoning_text", text: "thinking" }],
    status: "in_progress",
  },
};

const sealedReasoning: ChatRow = {
  id: "item-session-1-1",
  streaming: false,
  item: {
    type: "reasoning",
    id: "rs_1",
    summary: [{ type: "summary_text", text: "thinking" }],
    content: [{ type: "reasoning_text", text: "thinking" }],
    status: "completed",
  },
};

const liveTool: ChatRow = {
  id: "live-fc_1",
  streaming: true,
  item: {
    type: "function_call",
    id: "fc_1",
    call_id: "call_1",
    name: "grep",
    arguments: "{\"pattern\":\"foo\"}",
    status: "completed",
  },
};

afterEach(() => {
  cleanup();
  grantPermission.mockClear();
});

describe("MessageList permission card", () => {
  it("renders permission_request inline inside MessageList (not a fullscreen overlay)", () => {
    const { container } = render(
      <MessageList
        messages={[oneMessage]}
        loadingHistory={false}
        canLoadMore={false}
        onLoadMore={() => {}}
        userDetailBefore={0}
        isRunning={true}
        scrollRef={makeScrollRef()}
        sessionId="session-1"
      />,
    );

    const list = screen.getByTestId("message-list");
    const card = screen.getByTestId("permission-card");

    expect(list.contains(card)).toBe(true);
    expect(container.querySelector(".fixed.inset-0")).toBeNull();
    expect(card.className).not.toMatch(/\bfixed\b/);
    expect(card.textContent).toMatch(/bash/);
    expect(card.textContent).toMatch(/Run bash command/);
  });

  it("wires Allow once to grantPermission", async () => {
    const user = userEvent.setup();
    render(
      <MessageList
        messages={[oneMessage]}
        loadingHistory={false}
        canLoadMore={false}
        onLoadMore={() => {}}
        userDetailBefore={0}
        isRunning={true}
        scrollRef={makeScrollRef()}
        sessionId="session-1"
      />,
    );

    await user.click(screen.getByRole("button", { name: "Allow once" }));
    expect(grantPermission).toHaveBeenCalledWith("session-1", true, false);
  });
});

describe("MessageList process group across seal", () => {
  it("keeps the process FoldCard expanded when the first row seals live→buffer", () => {
    const { rerender } = render(
      <MessageList
        messages={[liveReasoning, liveTool]}
        loadingHistory={false}
        canLoadMore={false}
        onLoadMore={() => {}}
        userDetailBefore={0}
        isRunning={true}
        scrollRef={makeScrollRef()}
        sessionId="session-1"
      />,
    );

    const header = screen.getByRole("button", { name: /1 reasoning \+ 1 tool call/i });
    expect(header.getAttribute("aria-expanded")).toBe("true");

    rerender(
      <MessageList
        messages={[sealedReasoning, liveTool]}
        loadingHistory={false}
        canLoadMore={false}
        onLoadMore={() => {}}
        userDetailBefore={0}
        isRunning={true}
        scrollRef={makeScrollRef()}
        sessionId="session-1"
      />,
    );

    const headerAfterSeal = screen.getByRole("button", {
      name: /1 reasoning \+ 1 tool call/i,
    });
    // Same DOM node ⇒ ProcessGroup/FoldCard did not remount on seal.
    expect(headerAfterSeal).toBe(header);
    expect(headerAfterSeal.getAttribute("aria-expanded")).toBe("true");
  });
});
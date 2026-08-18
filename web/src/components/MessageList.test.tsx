import React from "react";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { MessageList, ProcessGroup, rowsToNodes } from "./MessageList";
import type { ChatRow } from "../api/adapter";
import { useBashStore } from "../stores/bashStore";

const grantPermission = vi.fn();

class ResizeObserverStub {
  observe() {}
  unobserve() {}
  disconnect() {}
}
vi.stubGlobal("ResizeObserver", ResizeObserverStub);

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
    scrollToIndex: () => {},
    isAtEnd: () => true,
    options: {},
  }),
}));

vi.mock("../stores/turnStore", () => ({
  useTurnStore: (selector: (state: Record<string, unknown>) => unknown) =>
    selector({
      byId: new Map([
        ["session-1", {
          pendingPermission: null,
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
  useBashStore.getState().reset();
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

    const header = screen.getByRole("button", { name: /1 reasoning, 1 tool/i });
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
      name: /1 reasoning, 1 tool/i,
    });
    // Same DOM node ⇒ ProcessGroup/FoldCard did not remount on seal.
    expect(headerAfterSeal).toBe(header);
    expect(headerAfterSeal.getAttribute("aria-expanded")).toBe("true");
  });
});

describe("ProcessGroup header buckets", () => {
  it("shows icon counts per category and excludes wait_shell from bash bucket", () => {
    const rows: ChatRow[] = [
      liveReasoning,
      {
        id: "fc_bash",
        streaming: false,
        item: {
          type: "function_call",
          id: "fc_bash",
          call_id: "call_bash",
          name: "bash",
          arguments: JSON.stringify({ command: "echo hi" }),
          status: "completed",
        },
      },
      {
        id: "fc_edit",
        streaming: false,
        item: {
          type: "function_call",
          id: "fc_edit",
          call_id: "call_edit",
          name: "edit",
          arguments: JSON.stringify({ file_path: "a.ts" }),
          status: "completed",
        },
      },
      liveTool,
      {
        id: "fc_wait",
        streaming: false,
        item: {
          type: "function_call",
          id: "fc_wait",
          call_id: "call_wait",
          name: "wait_shell",
          arguments: JSON.stringify({ id: "bg_a", sec: 5 }),
          status: "completed",
        },
      },
    ];
    const nodes = rowsToNodes(rows, true);
    const now = Date.now();
    useBashStore.getState().applySnapshot("session-1", {
      jobs: [],
      waits: [
        {
          call_id: "call_wait",
          watching_id: "bg_a",
          started_at_ms: now,
          deadline_ms: now + 5_000,
        },
      ],
    });
    render(
      <ProcessGroup
        nodes={nodes}
        streaming={true}
        sessionId="session-1"
        bubbleKey="bubble-1"
        groupIndex={0}
      />,
    );

    const header = screen.getByRole("button", {
      name: "1 reasoning, 1 bash, 1 edit, 1 tool",
    });
    expect(header).toBeTruthy();
    expect(header.textContent).toContain("×1");
    expect(screen.queryByRole("button", { name: /wait_shell/i })).toBeNull();
    expect(screen.getByTestId("wait-elapsed")).toBeTruthy();
  });
});

describe("MessageList system reminder", () => {
  it("renders a one-line notice without revert or reminder body", () => {
    const reminder: ChatRow = {
      id: "item-session-1-0",
      item: {
        type: "message",
        role: "user",
        id: "msg_rem",
        status: "completed",
        content: [
          {
            type: "input_text",
            text: "<system-reminder>\nThe user stopped background bash bg_a (Kill).\n</system-reminder>",
          },
        ],
      },
    };
    render(
      <MessageList
        messages={[reminder]}
        loadingHistory={false}
        canLoadMore={false}
        onLoadMore={() => {}}
        userDetailBefore={0}
        isRunning={false}
        scrollRef={makeScrollRef()}
        sessionId="session-1"
      />,
    );
    expect(screen.getByRole("status", { name: "Terminal killed" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Revert to here" })).toBeNull();
    expect(screen.queryByText(/system-reminder/)).toBeNull();
    expect(screen.queryByText(/bg_a/)).toBeNull();
  });
});

describe("MessageList stick intent", () => {
  it("unsticks only when the human wheels up on the list", () => {
    const onStickChange = vi.fn();
    const scrollRef = React.createRef<HTMLDivElement>();
    render(
      <div ref={scrollRef} data-testid="scroller">
        <MessageList
          messages={[liveReasoning]}
          loadingHistory={false}
          canLoadMore={false}
          onLoadMore={() => {}}
          userDetailBefore={0}
          isRunning={true}
          scrollRef={scrollRef}
          sessionId="session-1"
          onStickChange={onStickChange}
        />
      </div>,
    );
    fireEvent.wheel(screen.getByTestId("scroller"), { deltaY: -40 });
    expect(onStickChange).toHaveBeenCalledWith(false);
  });
});
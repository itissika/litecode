import React from "react";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { MessageList, ProcessGroup, rowsToNodes } from "./MessageList";
import type { HumanRow } from "../api/types";
import { useBashStore } from "../stores/bashStore";
import { useMessageStore } from "../stores/messageStore";
import { clearFoldCardOpen } from "./foldCardState";

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

const turnState = {
  byId: new Map<
    string,
    { pendingPermission: unknown; compacting: boolean; turnPhase: string | null }
  >([
    [
      "session-1",
      {
        pendingPermission: null,
        compacting: false,
        turnPhase: null,
      },
    ],
  ]),
};

vi.mock("../stores/turnStore", () => ({
  useTurnStore: (selector: (state: Record<string, unknown>) => unknown) =>
    selector(turnState),
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

const liveReasoning: HumanRow = {
  seq: 0,
  kind: "item/assistant",

  streaming: true,
  body: {
    type: "reasoning",
    id: "rs_1",
    summary: [{ type: "summary_text", text: "thinking" }],
    content: [{ type: "reasoning_text", text: "thinking" }],
    status: "in_progress",
  },
};

const sealedReasoning: HumanRow = {
  seq: 0,
  kind: "item/assistant",

  streaming: false,
  body: {
    type: "reasoning",
    id: "rs_1",
    summary: [{ type: "summary_text", text: "thinking" }],
    content: [{ type: "reasoning_text", text: "thinking" }],
    status: "completed",
  },
};

const liveTool: HumanRow = {
  seq: 2,
  kind: "item/tool_call",

  streaming: true,
  body: {
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
  clearFoldCardOpen("session-1");
  const slice = turnState.byId.get("session-1");
  if (slice) {
    slice.compacting = false;
    slice.turnPhase = null;
  }
});

describe("MessageList G5 historical FoldCard", () => {
  it("does not open a completed process group because the session is running", () => {
    const completedReasoning: HumanRow = {
      ...sealedReasoning,
      seq: 0,
    };
    const completedTool: HumanRow = {
      seq: 2,
      kind: "item/tool_call",

      streaming: false,
      body: {
        type: "function_call",
        id: "fc_hist",
        call_id: "call_hist",
        name: "grep",
        arguments: "{\"pattern\":\"foo\"}",
        status: "completed",
      },
    };
    const completedToolOutput: HumanRow = {
      seq: 3,
      kind: "item/tool_result",
      streaming: false,
      body: {
        type: "function_call_output",
        call_id: "call_hist",
        output: "ok",
      },
    };
    const finalMessage: HumanRow = {
      seq: 4,
      kind: "item/assistant",
      streaming: false,
      body: {
        type: "message",
        id: "msg_hist",
        role: "assistant",
        status: "completed",
        content: [{ type: "output_text", text: "done", annotations: [] }],
      },
    };
    render(
      <MessageList
        messages={[completedReasoning, completedTool, completedToolOutput, finalMessage]}
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
    expect(header.getAttribute("aria-expanded")).toBe("false");
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

  it("keeps the process FoldCard expanded after the call seals until output arrives", () => {
    const sealedCall: HumanRow = {
      ...liveTool,
      streaming: false,
    };
    const { rerender } = render(
      <MessageList
        messages={[sealedReasoning, sealedCall]}
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
        messages={[
          sealedReasoning,
          sealedCall,
          {
            seq: 3,
            kind: "item/tool_result",
            streaming: false,
            body: {
              type: "function_call_output",
              call_id: "call_1",
              output: "hits",
            },
          },
        ]}
        loadingHistory={false}
        canLoadMore={false}
        onLoadMore={() => {}}
        userDetailBefore={0}
        isRunning={true}
        scrollRef={makeScrollRef()}
        sessionId="session-1"
      />,
    );
    expect(
      screen.getByRole("button", { name: /1 reasoning, 1 tool/i }).getAttribute("aria-expanded"),
    ).toBe("true");

    rerender(
      <MessageList
        messages={[
          sealedReasoning,
          sealedCall,
          {
            seq: 3,
            kind: "item/tool_result",
            streaming: false,
            body: {
              type: "function_call_output",
              call_id: "call_1",
              output: "hits",
            },
          },
          {
            seq: 4,
            kind: "item/assistant",
            streaming: false,
            body: {
              type: "message",
              id: "msg_1",
              role: "assistant",
              status: "completed",
              content: [{ type: "output_text", text: "done", annotations: [] }],
            },
          },
        ]}
        loadingHistory={false}
        canLoadMore={false}
        onLoadMore={() => {}}
        userDetailBefore={0}
        isRunning={true}
        scrollRef={makeScrollRef()}
        sessionId="session-1"
      />,
    );
    expect(
      screen.getByRole("button", { name: /1 reasoning, 1 tool/i }).getAttribute("aria-expanded"),
    ).toBe("false");
  });
});

describe("ProcessGroup header buckets", () => {
  it("shows icon counts per category and excludes wait_shell from bash bucket", () => {
    const rows: HumanRow[] = [
      liveReasoning,
      {
        seq: 10,
        kind: "item/tool_call",

        streaming: false,
        body: {
          type: "function_call",
          id: "fc_bash",
          call_id: "call_bash",
          name: "bash",
          arguments: JSON.stringify({ command: "echo hi" }),
          status: "completed",
        },
      },
      {
        seq: 11,
        kind: "item/tool_call",

        streaming: false,
        body: {
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
        seq: 12,
        kind: "item/tool_call",

        streaming: false,
        body: {
          type: "function_call",
          id: "fc_wait",
          call_id: "call_wait",
          name: "wait_shell",
          arguments: JSON.stringify({ id: "bg_a", sec: 5 }),
          status: "completed",
        },
      },
    ];
    const nodes = rowsToNodes(rows);
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
        autoOpen={true}
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

describe("MessageList reminder rows", () => {
  it("hides explicit reminder log rows without inspecting their text", () => {
    const reminder: HumanRow = {
      seq: 0,
      kind: "reminder/job_exit",
      body: { job_id: "bg_a", reason: "kill", text: "hidden reminder" },
    };
    render(
      <MessageList messages={[reminder]} loadingHistory={false} canLoadMore={false}
        onLoadMore={() => {}} userDetailBefore={0} isRunning={false}
        scrollRef={makeScrollRef()} sessionId="session-1" />,
    );
    expect(screen.queryByText(/hidden reminder/)).toBeNull();
    expect(screen.queryByRole("button", { name: "Revert to here" })).toBeNull();
  });

  it("does not render unknown kinds even when body looks like an assistant message", () => {
    const unknown = {
      seq: 1,
      kind: "future/widget",
      body: {
        type: "message",
        role: "assistant",
        id: "ghost",
        status: "completed",
        content: [{ type: "output_text", text: "do not render", annotations: [] }],
      },
    } as unknown as HumanRow;
    expect(rowsToNodes([unknown])).toEqual([]);
    render(
      <MessageList messages={[unknown]} loadingHistory={false} canLoadMore={false}
        onLoadMore={() => {}} userDetailBefore={0} isRunning={false}
        scrollRef={makeScrollRef()} sessionId="session-1" />,
    );
    expect(screen.queryByText("do not render")).toBeNull();
  });

  it("revert k uses server userDetailBefore on a partial window", () => {
    const spy = vi.spyOn(useMessageStore.getState(), "revertToUserAnchor");
    const user: HumanRow = {
      seq: 12,
      kind: "item/user",
      body: {
        type: "message",
        role: "user",
        content: [{ type: "input_text", text: "later ask" }],
      },
    };
    render(
      <MessageList messages={[user]} loadingHistory={false} canLoadMore={false}
        onLoadMore={() => {}} userDetailBefore={4} isRunning={false}
        scrollRef={makeScrollRef()} sessionId="session-1" />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Revert to here" }));
    expect(spy).toHaveBeenCalledWith("session-1", 4);
    spy.mockRestore();
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

describe("MessageList compacting now marker", () => {
  const slice = () => turnState.byId.get("session-1")!;

  it("shows the compacting wave line while a manual compaction is in progress", () => {
    slice().compacting = true;
    const { rerender } = render(
      <MessageList
        messages={[liveReasoning]}
        loadingHistory={false}
        canLoadMore={false}
        onLoadMore={() => {}}
        userDetailBefore={0}
        isRunning={false}
        scrollRef={makeScrollRef()}
        sessionId="session-1"
      />,
    );
    expect(screen.getByTestId("compacting-now")).toBeTruthy();
    // Same per-character wave animation as the wait-shell text.
    expect(document.querySelectorAll(".wait-wave-char").length).toBeGreaterThan(0);

    slice().compacting = false;
    rerender(
      <MessageList
        messages={[liveReasoning]}
        loadingHistory={false}
        canLoadMore={false}
        onLoadMore={() => {}}
        userDetailBefore={0}
        isRunning={false}
        scrollRef={makeScrollRef()}
        sessionId="session-1"
      />,
    );
    expect(screen.queryByTestId("compacting-now")).toBeNull();
  });

  it("shows the compacting line for auto compaction via turnPhase", () => {
    slice().turnPhase = "compacting";
    render(
      <MessageList
        messages={[liveReasoning]}
        loadingHistory={false}
        canLoadMore={false}
        onLoadMore={() => {}}
        userDetailBefore={0}
        isRunning={true}
        scrollRef={makeScrollRef()}
        sessionId="session-1"
      />,
    );
    expect(screen.getByTestId("compacting-now")).toBeTruthy();
    expect(document.querySelectorAll(".wait-wave-char").length).toBeGreaterThan(0);
  });
});
import React from "react";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { MessageList, NodeView, ProcessGroup, rowsToNodes } from "./MessageList";
import type { HumanRow } from "../api/types";
import { userTextItem } from "../api/adapter";
import { useBashStore } from "../stores/bashStore";
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
    selector({ project: "/test/project", byId: new Map() }),
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
      body: userTextItem("hidden reminder"),
    };
    render(
      <MessageList messages={[reminder]} loadingHistory={false} canLoadMore={false}
        onLoadMore={() => {}} userDetailBefore={0} isRunning={false}
        scrollRef={makeScrollRef()} sessionId="session-1" />,
    );
    expect(screen.queryByText(/hidden reminder/)).toBeNull();
    expect(screen.getByRole("status", { name: "Background terminal exited" })).toBeTruthy();
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

  it("opens a user-message editor with the server-derived anchor", () => {
    const onEditAnchor = vi.fn();
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
        scrollRef={makeScrollRef()} sessionId="session-1" onEditAnchor={onEditAnchor} />,
    );
    fireEvent.click(screen.getByText("later ask"));
    expect(onEditAnchor).toHaveBeenCalledWith(
      expect.objectContaining({ userAnchorK: 4, draft: "later ask" }),
    );
  });

  it("does not open the editor for an unsealed optimistic user bubble, while earlier sealed bubbles still open", () => {
    const onEditAnchor = vi.fn();
    const sealed: HumanRow = {
      seq: 12,
      kind: "item/user",
      body: {
        type: "message",
        role: "user",
        content: [{ type: "input_text", text: "earlier ask" }],
      },
    };
    // Composer `start()` paints this with seq=-1 until buffer/item seals it.
    // After a silent fail the row stays pending, so the just-sent bubble is
    // not a revert target even though it looks like a sent user message.
    const pending: HumanRow = {
      seq: -1,
      kind: "item/user",
      body: {
        type: "message",
        role: "user",
        content: [{ type: "input_text", text: "just sent" }],
      },
    };
    render(
      <MessageList messages={[sealed, pending]} loadingHistory={false} canLoadMore={false}
        onLoadMore={() => {}} userDetailBefore={4} isRunning={true}
        scrollRef={makeScrollRef()} sessionId="session-1" onEditAnchor={onEditAnchor} />,
    );

    fireEvent.click(screen.getByText("earlier ask"));
    expect(onEditAnchor).toHaveBeenCalledWith(
      expect.objectContaining({ draft: "earlier ask", userAnchorK: 4 }),
    );
    onEditAnchor.mockClear();

    fireEvent.click(screen.getByText("just sent"));
    expect(onEditAnchor).not.toHaveBeenCalled();
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

  it("does not keep the compacting line after auto compact when only turnPhase is stuck", () => {
    slice().compacting = false;
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
    expect(screen.queryByTestId("compacting-now")).toBeNull();
  });
});

function bashRows(output: string): HumanRow[] {
  return [
    {
      seq: 1,
      kind: "item/tool_call",
      streaming: false,
      body: {
        type: "function_call",
        id: "fc_bash",
        call_id: "call_bash",
        name: "bash",
        arguments: JSON.stringify({ command: "sleep 8" }),
        status: "completed",
      },
    },
    {
      seq: 2,
      kind: "item/tool_result",
      streaming: false,
      body: {
        type: "function_call_output",
        call_id: "call_bash",
        output,
      },
    },
  ];
}

const RUNNING_DOC = `status: running
bash_id: bg_a
output_file: .litecode/bash/bg_a.output
`;

function seedBashJob(): void {
  useBashStore.getState().applySnapshot("session-1", {
    jobs: [
      {
        id: "bg_a",
        call_id: "call_bash",
        command_preview: "sleep 8",
        output_file: ".litecode/bash/bg_a.output",
        started_at_ms: Date.now(),
      },
    ],
    waits: [],
  });
}

describe("MessageList tool routing (inline row vs rich card)", () => {
  it("keeps a FOREGROUND bash on its rich card", () => {
    const node = rowsToNodes(bashRows(`exit_code: 0
all good
`))[0]!;
    const { container } = render(
      <NodeView node={node} projectRoot={null} sessionId="session-1" />,
    );

    // Still a collapsible tool card, not a single-line row. (The card body is
    // unmounted while collapsed — it is exercised directly in
    // BashToolView.test.tsx.)
    expect(container.querySelector(".foldcard-header")).toBeTruthy();
    expect(screen.queryByTestId("inline-bash-command")).toBeNull();
    expect(screen.queryByTestId("bash-console")).toBeNull();
  });

  it("routes a BACKGROUND bash to the single-line row", () => {
    seedBashJob();
    const node = rowsToNodes(bashRows(RUNNING_DOC))[0]!;
    const { container } = render(
      <NodeView node={node} projectRoot={null} sessionId="session-1" />,
    );

    expect(screen.getByTestId("inline-bash-command").textContent).toBe("sleep 8");
    expect(container.querySelector(".foldcard-header")).toBeNull();
    expect(screen.queryByTestId("bash-console")).toBeNull();
  });
});

describe("MessageList session-mount capsules route to single-line rows", () => {
  const capsule = (name: string, output: string): HumanRow[] => [
    {
      seq: 1,
      kind: "item/tool_call",
      streaming: false,
      body: {
        type: "function_call",
        id: "fc_cap",
        call_id: "call_cap",
        name,
        arguments: JSON.stringify({ action: "create" }),
        status: "completed",
      },
    },
    {
      seq: 2,
      kind: "item/tool_result",
      streaming: false,
      body: { type: "function_call_output", call_id: "call_cap", output },
    },
  ];

  it("renders todo as a summary row, never a card", () => {
    const node = rowsToNodes(
      capsule("todo", "OK. Status — pending: 2, in_progress: 1, completed: 3"),
    )[0]!;
    const { container } = render(<NodeView node={node} sessionId="session-1" />);

    expect(screen.getByTestId("inline-todo-summary").textContent).toBe(
      "1 active · 2 pending · 3 done",
    );
    expect(container.querySelector(".foldcard-header")).toBeNull();
  });

  it("renders plan as a summary row, never a card", () => {
    const node = rowsToNodes(
      capsule("plan", "Created plan at .litecode/plan/calm-river.md\nsaved."),
    )[0]!;
    const { container } = render(<NodeView node={node} sessionId="session-1" />);

    expect(screen.getByTestId("inline-plan-summary").textContent).toBe(
      ".litecode/plan/calm-river.md",
    );
    expect(container.querySelector(".foldcard-header")).toBeNull();
  });
});

describe("MessageList job_exit mark", () => {
  const exitRow: HumanRow = {
    seq: 9,
    kind: "reminder/job_exit",
    streaming: false,
    body: {
      type: "message",
      role: "user",
      content: [
        {
          type: "input_text",
          text: `<system-reminder>
Background bash bg_a exited with code 3.
output_file: .litecode/bash/bg_a.output
command: sleep 8
</system-reminder>`,
        },
      ],
    },
  };

  it("carries the exit detail from the reminder body into the mark", () => {
    const node = rowsToNodes([exitRow])[0]!;
    expect(node).toMatchObject({ kind: "job_exit", detail: "bg_a · exit code 3" });

    render(<NodeView node={node} />);
    expect(
      screen.getByText("background terminal exited · bg_a · exit code 3"),
    ).toBeTruthy();
  });

  it("falls back to the plain label when the body has no exit line", () => {
    const node = rowsToNodes([
      { ...exitRow, body: { type: "message", role: "user", content: [{ type: "input_text", text: "no detail" }] } } as HumanRow,
    ])[0]!;
    expect(node.kind === "job_exit" && node.detail).toBeFalsy();

    render(<NodeView node={node} />);
    expect(screen.getByText("background terminal exited")).toBeTruthy();
  });
});

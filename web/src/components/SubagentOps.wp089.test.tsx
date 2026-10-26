import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { SessionInfo } from "../api/types";
import { closePanel } from "../dockview/workbench/commands";
import {
  deleteRejectedMessage,
  readonlyRejectedMessage,
  DELETE_FAILED_MESSAGE,
} from "../lib/sessionErrors";
import { childStatusWord, deletedChildIds } from "../lib/subagentUi";
import { useConnectionStore } from "../stores/connectionStore";
import { useMessageStore } from "../stores/messageStore";
import {
  resetDeleteSessionInFlight,
  resetListSessionsInFlight,
  useSessionStore,
} from "../stores/sessionStore";
import { useToastStore } from "../stores/toastStore";
import { EMPTY_SLICE, useTurnStore } from "../stores/turnStore";
import { AgentChatInput } from "./AgentChatInput";
import { SessionStatusLine } from "./SessionStatusLine";
import { SubagentRosterPanel } from "./SubagentRosterPanel";
import { SubagentLaunchToolView } from "./toolviews/SubagentLaunchToolView";

vi.mock("../dockview/workbench/commands", async (orig) => ({
  ...(await orig<typeof import("../dockview/workbench/commands")>()),
  closePanel: vi.fn(),
}));
vi.mock("../lib/sessionPanelNav", async (orig) => ({
  ...(await orig<typeof import("../lib/sessionPanelNav")>()),
  openSubagentPanel: vi.fn(),
  openSessionPanel: vi.fn(),
}));

const PARENT = "parent-1";
const CHILD = "child-1";

function session(id: string, patch: Partial<SessionInfo> = {}): SessionInfo {
  return {
    id,
    project: "/p",
    updated_at: 1,
    preview: "",
    running: false,
    turn: null,
    agent_id: "worker",
    api_model_id: "m",
    ...patch,
  };
}

function rpcError(code: string, data: Record<string, unknown> = {}) {
  return Object.assign(new Error(code), {
    rpcCode: -32000,
    data: { code, ...data },
  });
}

let sendRpc: ReturnType<typeof vi.fn>;
function mockRpc(impl: (method: string, params?: unknown) => unknown) {
  sendRpc = vi.fn(async (method: string, params?: unknown) => impl(method, params));
  useConnectionStore.setState({ state: "connected", sendRpc } as never);
}
const toasts = () => useToastStore.getState().toasts;

beforeEach(() => {
  resetDeleteSessionInFlight();
  resetListSessionsInFlight();
  vi.mocked(closePanel).mockClear();
  useTurnStore.setState({ byId: new Map() });
  useMessageStore.setState({ bySession: new Map() });
  useToastStore.setState({ toasts: [] });
  useSessionStore.setState({
    sessions: [
      session(PARENT),
      session(CHILD, { parent_session_id: PARENT, parent_call_id: "call_1" }),
    ],
    deletedSessionIds: {},
  } as never);
  mockRpc(() => ({}));
});
afterEach(() => cleanup());

function setChild(patch: Partial<SessionInfo>) {
  useSessionStore.setState({
    sessions: [
      session(PARENT),
      session(CHILD, { parent_session_id: PARENT, parent_call_id: "call_1", ...patch }),
    ],
  } as never);
}

describe("WP-089 T1 子会话视图", () => {
  it("无输入框/发送；停止按钮仅在跑时可用并发 agent/cancel{child}", async () => {
    mockRpc(() => ({ cancelled: true }));
    const { rerender } = render(<AgentChatInput sessionId={CHILD} variant="subagent" />);
    expect(document.querySelector("textarea")).toBeNull();
    expect(screen.queryByTitle("Send")).toBeNull();
    expect(screen.queryByTestId("subagent-stop")).toBeNull();

    for (const status of ["running", "running_with_subagent", "stopping"] as const) {
      act(() => {
        setChild({ status, running: true });
        // The derived view subscribes to the child, so a live turn arrives as
        // `runState` too. That is when stop matters — it must not be disabled.
        useTurnStore.setState({
          byId: new Map([
            [
              CHILD,
              {
                ...EMPTY_SLICE,
                runState: status === "stopping" ? "cancelling" : "running",
              },
            ],
          ]),
        });
      });
      rerender(<AgentChatInput sessionId={CHILD} variant="subagent" />);
      expect((screen.getByTestId("subagent-stop") as HTMLButtonElement).disabled).toBe(false);
    }
    fireEvent.click(screen.getByTestId("subagent-stop"));
    expect(sendRpc).toHaveBeenCalledWith("agent/cancel", { session_id: CHILD });
  });
});

describe("WP-089 T2 cancelled:false", () => {
  it("中性 toast，不改状态", async () => {
    setChild({ status: "running", running: true });
    mockRpc(() => ({ cancelled: false }));
    useSessionStore.getState().stopSubagent(CHILD);
    await waitFor(() => expect(toasts()).toHaveLength(1));
    expect(toasts()[0]).toMatchObject({ message: "没有在运行的回合", variant: "info" });
    expect(useSessionStore.getState().sessions.find((s) => s.id === CHILD)?.status).toBe("running");
  });
  it("cancelled:true 不提示", async () => {
    mockRpc(() => ({ cancelled: true }));
    useSessionStore.getState().stopSubagent(CHILD);
    await Promise.resolve();
    await Promise.resolve();
    expect(toasts()).toHaveLength(0);
  });
});

describe("WP-089 T3 Workers 名单行操作", () => {
  it("在跑：只显示停止（含 running_with_subagent）", () => {
    setChild({ status: "running_with_subagent", running: true });
    render(<SubagentRosterPanel sessionId={PARENT} />);
    expect(screen.getByTestId("subagent-roster-stop")).toBeTruthy();
    expect(screen.queryByTestId("subagent-roster-delete")).toBeNull();
  });
  it("确认框取消不发 RPC，确认才发 session/delete；点行仍可跳转", () => {
    render(<SubagentRosterPanel sessionId={PARENT} />);
    const confirm = vi.spyOn(window, "confirm").mockReturnValueOnce(false);
    fireEvent.click(screen.getByTestId("subagent-roster-delete"));
    expect(sendRpc).not.toHaveBeenCalledWith("session/delete", expect.anything());
    confirm.mockReturnValueOnce(true);
    fireEvent.click(screen.getByTestId("subagent-roster-delete"));
    expect(sendRpc).toHaveBeenCalledWith("session/delete", { id: CHILD });
    confirm.mockRestore();
  });
  it("停止按钮发 agent/cancel{child}", () => {
    setChild({ status: "running", running: true });
    render(<SubagentRosterPanel sessionId={PARENT} />);
    fireEvent.click(screen.getByTestId("subagent-roster-stop"));
    expect(sendRpc).toHaveBeenCalledWith("agent/cancel", { session_id: CHILD });
  });
});

describe("WP-089 T4 deleted 事件", () => {
  it("关两种面板；记已删；名单/计数/卡片排除", () => {
    useMessageStore.getState().onSubagentBound(PARENT, {
      session_id: PARENT,
      call_id: "call_1",
      child_session_id: CHILD,
    });
    act(() =>
      useSessionStore.getState().onSessionLifecycle({
        session_id: CHILD,
        event: "deleted",
        turn: null,
        parent_session_id: PARENT,
      }),
    );
    expect(closePanel).toHaveBeenCalledWith(`agent-${CHILD}`);
    expect(closePanel).toHaveBeenCalledWith(`subagent-${CHILD}`);
    expect(useSessionStore.getState().deletedSessionIds[CHILD]).toBe(true);

    render(<SubagentRosterPanel sessionId={PARENT} />);
    expect(screen.getByText("No subagents")).toBeTruthy();
    cleanup();

    render(
      <SubagentLaunchToolView
        name="subagent_launch"
        input={{ agent: "worker" }}
        status="ok"
        call_id="call_1"
        sessionId={PARENT}
      />,
    );
    const line = screen.getByTestId("subagent-launch-line");
    expect(line.tagName).toBe("DIV");
    expect(line.textContent).toContain("已删除");
  });
  it("级联：每个子各一条 deleted 都各自处理", () => {
    for (const id of ["c-a", "c-b"]) {
      useSessionStore.getState().onSessionLifecycle({
        session_id: id,
        event: "deleted",
        turn: null,
        parent_session_id: PARENT,
      });
    }
    expect(Object.keys(useSessionStore.getState().deletedSessionIds)).toEqual(["c-a", "c-b"]);
  });
});

describe("WP-089 T5 重连统一拉一次 session/list", () => {
  it("并发多次 listSessions 只发一个请求；返回后才可再发", async () => {
    let resolve!: (v: unknown) => void;
    mockRpc(() => new Promise((r) => (resolve = r)));
    useSessionStore.getState().listSessions();
    useSessionStore.getState().listSessions();
    expect(sendRpc).toHaveBeenCalledTimes(1);
    resolve({
      sessions: [session(PARENT, { status: "running_with_subagent", running: true })],
    });
    await waitFor(() =>
      expect(useSessionStore.getState().sessions[0]?.status).toBe("running_with_subagent"),
    );
    useSessionStore.getState().listSessions();
    expect(sendRpc).toHaveBeenCalledTimes(2);
  });
});

describe("WP-089 T6 lifecycle 原始 status", () => {
  const fire = (event: "turn_started" | "turn_finished", status?: SessionInfo["status"]) =>
    useSessionStore.getState().onSessionLifecycle({
      session_id: PARENT,
      event,
      turn: null,
      ...(status ? { status } : {}),
    });
  const parent = () => useSessionStore.getState().sessions.find((s) => s.id === PARENT)!;
  it("turn_finished 带 running_with_subagent 保持", () => {
    fire("turn_finished", "running_with_subagent");
    expect(parent().status).toBe("running_with_subagent");
    expect(parent().running).toBe(true);
  });
  it("turn_started 带 stopping 用原值", () => {
    fire("turn_started", "stopping");
    expect(parent().status).toBe("stopping");
  });
  it("无 status 回退：started→running, finished→idle", () => {
    fire("turn_started");
    expect(parent().status).toBe("running");
    fire("turn_finished");
    expect(parent().status).toBe("idle");
    expect(parent().running).toBe(false);
  });
});

describe("WP-089 T7 回归：主会话删除", () => {
  it("在跑不删；idle 删除走 session/delete 并只关 agent 面板", async () => {
    useSessionStore.setState({ sessions: [session("main", { running: true })] } as never);
    useSessionStore.getState().deleteSession("main");
    expect(sendRpc).not.toHaveBeenCalled();
    useSessionStore.setState({ sessions: [session("main")] } as never);
    useSessionStore.getState().deleteSession("main");
    expect(sendRpc).toHaveBeenCalledWith("session/delete", { id: "main" });
    // Not optimistic: the row survives until the server confirms.
    expect(useSessionStore.getState().sessions).toHaveLength(1);
    expect(closePanel).not.toHaveBeenCalled();
    await waitFor(() => expect(closePanel).toHaveBeenCalledWith("agent-main"));
    expect(useSessionStore.getState().sessions).toHaveLength(0);
  });

  it("删除提交后，晚到的旧 session/list 不会复活该行", async () => {
    useSessionStore.setState({ sessions: [session("main")] } as never);
    useSessionStore.getState().deleteSession("main");
    await waitFor(() => expect(useSessionStore.getState().sessions).toHaveLength(0));

    useSessionStore.getState().onSessionList([session("main"), session("other")]);
    expect(useSessionStore.getState().sessions.map((s) => s.id)).toEqual(["other"]);
  });

  it("删除提交后，晚到的 lifecycle 不会 upsert 回来", async () => {
    useSessionStore.setState({ sessions: [session("main")] } as never);
    useSessionStore.getState().deleteSession("main");
    await waitFor(() =>
      expect(useSessionStore.getState().deletedSessionIds.main).toBe(true),
    );
    useSessionStore.getState().onSessionLifecycle({
      session_id: "main",
      event: "preview_updated",
      turn: null,
      preview: "late",
    });
    expect(useSessionStore.getState().sessions.some((s) => s.id === "main")).toBe(false);
  });

  it("同一会话的删除在飞行中不重复发送", async () => {
    let finish!: (v: unknown) => void;
    mockRpc((method) => {
      if (method === "session/delete") {
        return new Promise((resolve) => {
          finish = resolve;
        });
      }
      return {};
    });
    useSessionStore.setState({ sessions: [session("main")] } as never);
    useSessionStore.getState().deleteSession("main");
    useSessionStore.getState().deleteSession("main");
    expect(sendRpc).toHaveBeenCalledTimes(1);
    finish({});
    await waitFor(() => expect(useSessionStore.getState().sessions).toHaveLength(0));
  });
});

describe("WP-089 T8 墓碑 subagent_binding_states", () => {
  const load = (states?: Record<string, { child_session_id: string; deleted: boolean }>) =>
    useMessageStore.getState().onBufferLoaded(PARENT, {
      session_id: PARENT,
      from_seq: 0,
      to_seq: 0,
      events: [],
      subagent_bindings: { call_1: CHILD },
      ...(states ? { subagent_binding_states: states } : {}),
    });
  const launch = () =>
    render(
      <SubagentLaunchToolView
        name="subagent_launch"
        input={{ agent: "worker" }}
        status="ok"
        call_id="call_1"
        sessionId={PARENT}
      />,
    );
  it("buffer load deleted=true → 已删除不可点，名单/计数排除", () => {
    load({ call_1: { child_session_id: CHILD, deleted: true } });
    launch();
    expect(screen.getByTestId("subagent-launch-line").tagName).toBe("DIV");
    expect(screen.getByText("已删除")).toBeTruthy();
    cleanup();
    render(<SubagentRosterPanel sessionId={PARENT} />);
    expect(screen.getByText("No subagents")).toBeTruthy();
  });
  it("snapshot 带 deleted=true 同样生效", () => {
    load();
    useMessageStore.getState().applySubagentBindingStates(PARENT, {
      call_1: { child_session_id: CHILD, deleted: true },
    });
    launch();
    expect(screen.getByText("已删除")).toBeTruthy();
  });
  it("deleted=false → 正常可点", () => {
    load({ call_1: { child_session_id: CHILD, deleted: false } });
    launch();
    expect(screen.getByTestId("subagent-launch-line").tagName).toBe("BUTTON");
    expect(screen.queryByText("已删除")).toBeNull();
  });
  it("字段缺失 / 无对应 call_id → 视为未删除", () => {
    load();
    expect(deletedChildIds(undefined, undefined).size).toBe(0);
    expect(
      deletedChildIds({ other: { child_session_id: "x", deleted: false } }, {}).has(CHILD),
    ).toBe(false);
    launch();
    expect(screen.getByTestId("subagent-launch-line").tagName).toBe("BUTTON");
    expect(childStatusWord(undefined, "ok", false)).toBe("accepted");
  });
  it("计数排除已删除子会话（对照：未删时有 0/1）", () => {
    load();
    const before = render(<SessionStatusLine sessionId={PARENT} />);
    expect(document.body.textContent ?? "").toMatch(/0\/1 running/);
    before.unmount();
    load({ call_1: { child_session_id: CHILD, deleted: true } });
    render(<SessionStatusLine sessionId={PARENT} />);
    expect(document.body.textContent ?? "").not.toMatch(/\d+\/1 running/);
  });
});

describe("WP-089 T9 删除被拒提示", () => {
  it("agent_already_running / descendant_running(有无 ids) / 未知 / 无码", () => {
    expect(deleteRejectedMessage(rpcError("agent_already_running"))).toBe(
      "会话正在运行，请先停止再删除",
    );
    expect(
      deleteRejectedMessage(rpcError("descendant_running", { running_session_ids: ["a", "b"] })),
    ).toBe("有 2 个子会话正在运行，请先停止再删除");
    expect(deleteRejectedMessage(rpcError("descendant_running"))).toBe(
      "有子会话正在运行，请先停止再删除",
    );
    expect(deleteRejectedMessage(rpcError("weird_code"))).toBe(DELETE_FAILED_MESSAGE);
    expect(deleteRejectedMessage(new Error("x"))).toBe(DELETE_FAILED_MESSAGE);
  });
  it("store 删除被拒：toast 对应文案并重拉列表", async () => {
    mockRpc((method) => {
      if (method === "session/delete") {
        throw rpcError("descendant_running", { running_session_ids: ["g"] });
      }
      return { sessions: [session(CHILD, { parent_session_id: PARENT })] };
    });
    useSessionStore.getState().deleteSession(CHILD);
    await waitFor(() => expect(toasts()).toHaveLength(1));
    expect(toasts()[0]!.message).toBe("有 1 个子会话正在运行，请先停止再删除");
    await waitFor(() => expect(sendRpc).toHaveBeenCalledWith("session/list"));
  });
});

describe("WP-089 T10 status_changed", () => {
  it("只更新 status；缺 status 忽略", () => {
    const lc = useSessionStore.getState().onSessionLifecycle;
    lc({ session_id: PARENT, event: "status_changed", turn: null, status: "running_with_subagent" });
    let p = useSessionStore.getState().sessions.find((s) => s.id === PARENT)!;
    expect(p.status).toBe("running_with_subagent");
    expect(p.running).toBe(true);
    lc({ session_id: PARENT, event: "status_changed", turn: null, status: "idle" });
    p = useSessionStore.getState().sessions.find((s) => s.id === PARENT)!;
    expect(p.status).toBe("idle");
    expect(p.running).toBe(false);
    lc({ session_id: PARENT, event: "status_changed", turn: null });
    expect(useSessionStore.getState().sessions.find((s) => s.id === PARENT)!.status).toBe("idle");
  });
});

describe("WP-089 T11 readonly 码", () => {
  it("subagent_session_readonly → 只读提示；其它 undefined", () => {
    expect(readonlyRejectedMessage(rpcError("subagent_session_readonly"))).toMatch(/只读/);
    expect(readonlyRejectedMessage(rpcError("agent_already_running"))).toBeUndefined();
  });
});

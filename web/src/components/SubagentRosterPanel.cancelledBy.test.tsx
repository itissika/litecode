import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { SessionInfo } from "../api/types";
import { useConnectionStore } from "../stores/connectionStore";
import { useMessageStore } from "../stores/messageStore";
import { resetListSessionsInFlight, useSessionStore } from "../stores/sessionStore";
import { useTurnStore } from "../stores/turnStore";
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

function child(patch: Partial<SessionInfo> = {}): SessionInfo {
  return {
    id: CHILD,
    project: "/p",
    updated_at: 1,
    preview: "",
    running: false,
    turn: null,
    agent_id: "worker",
    api_model_id: "m",
    parent_session_id: PARENT,
    parent_call_id: "call_1",
    ...patch,
  };
}
const parent: SessionInfo = { ...child(), id: PARENT, parent_session_id: null, parent_call_id: null };

function seed(patch: Partial<SessionInfo>) {
  useSessionStore.setState({ sessions: [parent, child(patch)], deletedSessionIds: {} } as never);
}
const mark = () => screen.queryByTestId("subagent-roster-cancelled-by-user");
const finished = () => screen.getByTestId("subagent-roster-finished").textContent;
const lc = (p: Record<string, unknown>) =>
  act(() => useSessionStore.getState().onSessionLifecycle({ session_id: CHILD, turn: null, ...p } as never));

let sendRpc: ReturnType<typeof vi.fn>;
beforeEach(() => {
  resetListSessionsInFlight();
  useTurnStore.setState({ byId: new Map() });
  useMessageStore.setState({ bySession: new Map() });
  sendRpc = vi.fn(async () => ({}));
  useConnectionStore.setState({ state: "connected", sendRpc } as never);
});
afterEach(() => cleanup());

describe("WP-097 Workers 名单 cancelled · by you", () => {
  it("1 cancelled + by:user → 显示标记", () => {
    seed({ last_turn_reason: "cancelled", last_turn_cancelled_by: "user" });
    render(<SubagentRosterPanel sessionId={PARENT} />);
    expect(finished()).toBe("cancelled · by you");
    expect(mark()).toBeTruthy();
  });

  it("2 cancelled + by:parent（含 stop_and_send 内部停）→ 仅 cancelled", () => {
    seed({ last_turn_reason: "cancelled", last_turn_cancelled_by: "parent" });
    render(<SubagentRosterPanel sessionId={PARENT} />);
    expect(finished()).toBe("cancelled");
    expect(mark()).toBeNull();
  });

  it("3 cancelled + 字段缺失 / null → 仅 cancelled", () => {
    seed({ last_turn_reason: "cancelled" });
    const r = render(<SubagentRosterPanel sessionId={PARENT} />);
    expect(finished()).toBe("cancelled");
    expect(mark()).toBeNull();
    r.unmount();
    seed({ last_turn_reason: "cancelled", last_turn_cancelled_by: null });
    render(<SubagentRosterPanel sessionId={PARENT} />);
    expect(finished()).toBe("cancelled");
    expect(mark()).toBeNull();
  });

  it("4 非 cancelled + by:user → 无标记", () => {
    for (const reason of ["completed", "error", "max_steps"]) {
      seed({ last_turn_reason: reason, last_turn_cancelled_by: "user" });
      const r = render(<SubagentRosterPanel sessionId={PARENT} />);
      expect(finished()).toBe(reason);
      expect(mark()).toBeNull();
      r.unmount();
    }
  });

  it("5 重连：session/list 带 by:user → 标记在（字段不进快照）", async () => {
    sendRpc = vi.fn(async () => ({
      sessions: [parent, child({ last_turn_reason: "cancelled", last_turn_cancelled_by: "user" })],
    }));
    useConnectionStore.setState({ sendRpc } as never);
    useSessionStore.setState({ sessions: [parent], deletedSessionIds: {} } as never);
    render(<SubagentRosterPanel sessionId={PARENT} />);
    await act(async () => {
      useSessionStore.getState().listSessions();
      await new Promise((r) => setTimeout(r, 0));
    });
    expect(mark()).toBeTruthy();

    // 重连：再拉一次 list（行内字段被清掉的旧值被 list 结果覆盖回来）
    act(() => seed({ last_turn_reason: "cancelled" }));
    expect(mark()).toBeNull();
    await act(async () => {
      useSessionStore.getState().listSessions();
      await new Promise((r) => setTimeout(r, 0));
    });
    expect(mark()).toBeTruthy();
  });

  it("6 新一轮：turn_started 清；turn_finished 无字段不残留；cancelled+user 再出现", () => {
    seed({ last_turn_reason: "cancelled", last_turn_cancelled_by: "user" });
    render(<SubagentRosterPanel sessionId={PARENT} />);
    expect(mark()).toBeTruthy();
    lc({ event: "turn_started" });
    lc({ event: "turn_finished", reason: "completed" });
    expect(mark()).toBeNull();
    expect(finished()).toBe("completed");
    lc({ event: "turn_finished", reason: "cancelled", cancelled_by: "user" });
    expect(mark()).toBeTruthy();
    // finished again with new reason but no field → no inheritance
    lc({ event: "turn_finished", reason: "cancelled" });
    expect(mark()).toBeNull();
    // 显式 null 清掉；事件上的旧键名 last_turn_cancelled_by 不被读取
    lc({ event: "turn_finished", reason: "cancelled", cancelled_by: "user" });
    lc({ event: "turn_finished", reason: "cancelled", cancelled_by: null });
    expect(mark()).toBeNull();
    lc({ event: "turn_finished", reason: "cancelled", last_turn_cancelled_by: "user" });
    expect(mark()).toBeNull();
  });

  it("7 回归：tool card 文字不变；停止/删除/计数不变", () => {
    seed({ last_turn_reason: "cancelled", last_turn_cancelled_by: "user" });
    useMessageStore.getState().onSubagentBound(PARENT, {
      session_id: PARENT,
      call_id: "call_1",
      child_session_id: CHILD,
    });
    render(
      <SubagentLaunchToolView name="subagent_launch" input={{ agent: "worker" }} status="ok" call_id="call_1" sessionId={PARENT} />,
    );
    expect(screen.getByTestId("subagent-launch-line").textContent).toContain("cancelled");
    expect(screen.getByTestId("subagent-launch-line").textContent).not.toContain("by you");
    cleanup();
    render(<SubagentRosterPanel sessionId={PARENT} />);
    expect(screen.queryByTestId("subagent-roster-stop")).toBeNull();
    expect(screen.getByTestId("subagent-roster-delete")).toBeTruthy();
    cleanup();
    render(<SessionStatusLine sessionId={PARENT} />);
    expect(document.body.textContent ?? "").toMatch(/0\/1 running/);
  });
});

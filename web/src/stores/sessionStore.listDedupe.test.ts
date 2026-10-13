import { beforeEach, describe, expect, it, vi } from "vitest";

import type { SessionInfo } from "../api/types";
import { useConnectionStore } from "./connectionStore";
import { resetListSessionsInFlight, useSessionStore } from "./sessionStore";

function session(id: string): SessionInfo {
  return {
    id,
    project: "/p",
    updated_at: 1,
    preview: "",
    running: false,
    turn: null,
    agent_id: "a",
    api_model_id: "m",
  };
}

type Deferred = { resolve: (v: unknown) => void; reject: (e: unknown) => void };
let calls: Deferred[];
let sendRpc: ReturnType<typeof vi.fn>;

beforeEach(() => {
  resetListSessionsInFlight();
  calls = [];
  sendRpc = vi.fn(
    () =>
      new Promise((resolve, reject) => {
        calls.push({ resolve, reject });
      }),
  );
  useConnectionStore.setState({ state: "connected", sendRpc } as never);
  useSessionStore.setState({ sessions: [], sessionListError: null } as never);
});

const flush = () => new Promise((r) => setTimeout(r, 0));
const list = () => useSessionStore.getState().listSessions();

describe("WP-093 listSessions 补拉一次", () => {
  it("L1 进行中再调 1 次 → 结束后恰好补 1 次", async () => {
    list();
    list();
    expect(sendRpc).toHaveBeenCalledTimes(1);
    calls[0]!.resolve({ sessions: [] });
    await flush();
    expect(sendRpc).toHaveBeenCalledTimes(2);
    calls[1]!.resolve({ sessions: [] });
    await flush();
    expect(sendRpc).toHaveBeenCalledTimes(2);
  });

  it("L2 进行中再调 3 次 → 仍只补 1 次", async () => {
    list();
    list();
    list();
    list();
    calls[0]!.resolve({ sessions: [] });
    await flush();
    calls[1]!.resolve({ sessions: [] });
    await flush();
    expect(sendRpc).toHaveBeenCalledTimes(2);
  });

  it("L3 无并发 → 只发 1 次", async () => {
    list();
    calls[0]!.resolve({ sessions: [session("a")] });
    await flush();
    expect(sendRpc).toHaveBeenCalledTimes(1);
    expect(useSessionStore.getState().sessions.map((s) => s.id)).toEqual(["a"]);
  });

  it("L4 首次失败且期间有新请求 → 仍补 1 次，成功后错误清空", async () => {
    list();
    list();
    calls[0]!.reject(new Error("boom"));
    await flush();
    expect(sendRpc).toHaveBeenCalledTimes(2);
    calls[1]!.resolve({ sessions: [session("a")] });
    await flush();
    expect(useSessionStore.getState().sessionListError).toBeNull();
    expect(useSessionStore.getState().sessions.map((s) => s.id)).toEqual(["a"]);
    expect(sendRpc).toHaveBeenCalledTimes(2);
  });

  it("L5 补拉结果覆盖首个结果（期间删除的会话消失）", async () => {
    list();
    list();
    calls[0]!.resolve({ sessions: [session("a"), session("gone")] });
    await flush();
    expect(useSessionStore.getState().sessions.map((s) => s.id).sort()).toEqual(["a", "gone"]);
    calls[1]!.resolve({ sessions: [session("a")] });
    await flush();
    expect(useSessionStore.getState().sessions.map((s) => s.id)).toEqual(["a"]);
  });
});

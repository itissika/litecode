import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

type Opts = { onConnectionChange: (s: string) => void };
const captured: { opts: Opts | null } = { opts: null };

vi.mock("../api/agentWs", () => ({
  AgentWsClient: class {
    constructor(opts: Opts) {
      captured.opts = opts;
    }
    connect() {}
    disconnect() {}
    send() {}
  },
}));
vi.mock("../api/auth", () => ({ getAuthToken: () => "test-token" }));

import { attachSiblingStores, useConnectionStore } from "./connectionStore";

const listSessions = vi.fn();

beforeEach(() => {
  captured.opts = null;
  listSessions.mockClear();
  attachSiblingStores({
    session: { getState: () => ({ listSessions }) } as never,
  });
  useConnectionStore.getState().init();
});

afterEach(() => {
  useConnectionStore.getState().destroy();
});

const change = (s: string) => captured.opts!.onConnectionChange(s);

describe("WP-093 重连接线：connected 时统一 listSessions", () => {
  it("R1 connected → listSessions 1 次", () => {
    expect(captured.opts).not.toBeNull();
    change("connected");
    expect(listSessions).toHaveBeenCalledTimes(1);
  });
  it("R2 reconnecting / disconnected 不调", () => {
    change("connecting");
    change("reconnecting");
    change("disconnected");
    expect(listSessions).not.toHaveBeenCalled();
  });
  it("R3 断开后再 connected 再调 1 次", () => {
    change("connected");
    change("reconnecting");
    change("connected");
    expect(listSessions).toHaveBeenCalledTimes(2);
  });
});

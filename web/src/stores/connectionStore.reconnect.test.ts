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
import { useEngineStore } from "./engineStore";
import {
  resetCentreSettleForTests,
  settleCentre,
  whenCentreSettled,
} from "../lib/centreSettle";

const listSessions = vi.fn();

beforeEach(() => {
  captured.opts = null;
  listSessions.mockClear();
  resetCentreSettleForTests();
  attachSiblingStores({
    session: { getState: () => ({ listSessions, onHello: vi.fn() }) } as never,
  });
  useConnectionStore.getState().init();
});

afterEach(() => {
  useConnectionStore.getState().destroy();
  resetCentreSettleForTests();
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

const flush = () => new Promise((r) => setTimeout(r, 0));

describe("WP-170 startup order: centre settle on transport failure (path C)", () => {
  it("C1 a failed socket (reconnecting, no hello) settles the centre", async () => {
    let done = false;
    void whenCentreSettled().then(() => {
      done = true;
    });
    change("connecting");
    await flush();
    expect(done).toBe(false);
    change("reconnecting");
    await flush();
    expect(done).toBe(true);
  });

  it("C2 disconnected also settles; a later reconnect does not wait again", async () => {
    change("disconnected");
    await whenCentreSettled();
    const ensureLoaded = vi.fn(async () => {});
    useEngineStore.setState({ ensureLoaded });
    change("connected");
    useConnectionStore.getState().dispatchEnvelope({
      method: "server/hello",
      params: { workspace_id: "w", project: "p" },
    } as never);
    await flush();
    await flush();
    expect(ensureLoaded).toHaveBeenCalledTimes(1);
  });

  it("C3 hello does not fetch engines until the centre settles", async () => {
    const ensureLoaded = vi.fn(async () => {});
    useEngineStore.setState({ ensureLoaded });
    change("connected");
    useConnectionStore.getState().dispatchEnvelope({
      method: "server/hello",
      params: { workspace_id: "w", project: "p" },
    } as never);
    await flush();
    expect(ensureLoaded).not.toHaveBeenCalled();
    settleCentre();
    await flush();
    await flush();
    expect(ensureLoaded).toHaveBeenCalledTimes(1);
  });
});

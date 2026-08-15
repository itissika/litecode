import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { AgentWsClient } from "./agentWs";

/** Capture the URL passed to `new WebSocket`. */
let wsUrl: string | null = null;
let sendSpy: ReturnType<typeof vi.fn> | null = null;

class MockWebSocket {
  static OPEN = 1;
  static CLOSED = 3;
  readyState = 0;
  onopen: (() => void) | null = null;
  onmessage: ((ev: { data: unknown }) => void) | null = null;
  onerror: (() => void) | null = null;
  onclose: (() => void) | null = null;
  constructor(url: string) {
    wsUrl = url;
    sendSpy = vi.fn();
  }
  send(_payload: string): void {
    sendSpy?.();
  }
  close(): void {
    this.readyState = 3;
  }
}

beforeEach(() => {
  wsUrl = null;
  sendSpy = null;
  vi.stubGlobal("WebSocket", MockWebSocket);
  // Neutralize the handshake/reconnect timers so tests don't leave pending work.
  vi.spyOn(globalThis, "setTimeout").mockImplementation((() => 0) as never);
  vi.spyOn(globalThis, "clearTimeout").mockImplementation((() => undefined) as never);
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe("AgentWsClient buildConnectUrl (G3)", () => {
  it("keeps the auth token in the WS handshake query and preserves existing params", () => {
    const client = new AgentWsClient({
      url: "ws://127.0.0.1:7483/ws?session=s1",
      authToken: "tok-123",
      onEnvelope: () => {},
    });
    client.connect();
    expect(wsUrl).not.toBeNull();
    const url = new URL(wsUrl!);
    expect(url.searchParams.get("token")).toBe("tok-123");
    expect(url.searchParams.get("session")).toBe("s1");
  });
});

describe("AgentWsClient send error surfacing (FE-10)", () => {
  it("reports an explicit error when the socket is not open", () => {
    const onError = vi.fn();
    const client = new AgentWsClient({
      url: "ws://127.0.0.1:7483/ws",
      onEnvelope: () => {},
      onError,
    });
    // No connect() → ws is null → not open.
    client.send("auth", { token: "x" });
    expect(onError).toHaveBeenCalledWith("WebSocket not connected");
  });

  it("reports an explicit error when ws.send throws", () => {
    const onError = vi.fn();
    const client = new AgentWsClient({
      url: "ws://127.0.0.1:7483/ws",
      onEnvelope: () => {},
      onError,
    });
    // Force the socket to appear open, then make send() throw.
    const throwingWs = new MockWebSocket("ws://x");
    throwingWs.readyState = 1; // OPEN
    (client as unknown as { ws: MockWebSocket }).ws = throwingWs;
    sendSpy!.mockImplementation(() => {
      throw new Error("socket closing");
    });

    client.send("auth", { token: "x" });
    expect(onError).toHaveBeenCalledWith(
      "WebSocket send failed: socket closing",
    );
  });
});

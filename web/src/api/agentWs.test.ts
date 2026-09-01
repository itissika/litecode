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

describe("AgentWsClient incoming frames", () => {
  const workspaceChanged = {
    jsonrpc: "2.0",
    method: "workspace/changed",
    params: { kind: "modified", paths: ["src/a.rs"] },
  };
  const workspaceChangedJson = JSON.stringify(workspaceChanged);

  function connectedClient(onEnvelope: (env: unknown) => void, onError: (e: string) => void) {
    const client = new AgentWsClient({
      url: "ws://127.0.0.1:7483/ws",
      onEnvelope,
      onError,
    });
    client.connect();
    const ws = (client as unknown as { ws: MockWebSocket }).ws;
    expect(ws.onmessage).toBeTypeOf("function");
    return ws;
  }

  it("dispatches a complete notification with no trailing newline", () => {
    const onEnvelope = vi.fn();
    const onError = vi.fn();
    const ws = connectedClient(onEnvelope, onError);
    ws.onmessage?.({ data: workspaceChangedJson });
    expect(onError).not.toHaveBeenCalled();
    expect(onEnvelope).toHaveBeenCalledWith(workspaceChanged);
  });

  it("does not toast a split workspace/changed frame", () => {
    const onEnvelope = vi.fn();
    const onError = vi.fn();
    const ws = connectedClient(onEnvelope, onError);
    const cut = workspaceChangedJson.indexOf('"paths"');
    expect(cut).toBeGreaterThan(0);
    ws.onmessage?.({ data: workspaceChangedJson.slice(0, cut) });
    expect(onError).not.toHaveBeenCalled();
    expect(onEnvelope).not.toHaveBeenCalled();
    ws.onmessage?.({ data: workspaceChangedJson.slice(cut) });
    expect(onError).not.toHaveBeenCalled();
    expect(onEnvelope).toHaveBeenCalledWith(workspaceChanged);
  });
});

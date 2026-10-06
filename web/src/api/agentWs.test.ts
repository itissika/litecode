import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { AgentWsClient } from "./agentWs";

/** Capture the URL passed to `new WebSocket`. */
let wsUrl: string | null = null;
let sendSpy: ReturnType<typeof vi.fn> | null = null;
let socketsCreated = 0;

class MockWebSocket {
  static OPEN = 1;
  static CLOSED = 3;
  readyState = 0;
  onopen: (() => void) | null = null;
  onmessage: ((ev: { data: unknown }) => void) | null = null;
  onerror: (() => void) | null = null;
  onclose: (() => void) | null = null;
  constructor(url: string) {
    socketsCreated += 1;
    wsUrl = url;
    sendSpy = vi.fn();
  }
  send(_payload: string): void {
    sendSpy?.();
  }
  close(): void {
    if (this.readyState === MockWebSocket.CLOSED) return;
    this.readyState = MockWebSocket.CLOSED;
    this.onclose?.();
  }
}

const HELLO = JSON.stringify({
  jsonrpc: "2.0",
  method: "server/hello",
  params: { project: "p", workspace_id: "w" },
});

function openAndHello(client: AgentWsClient): MockWebSocket {
  client.connect();
  const ws = (client as unknown as { ws: MockWebSocket }).ws;
  ws.readyState = MockWebSocket.OPEN;
  ws.onopen?.();
  ws.onmessage?.({ data: HELLO });
  return ws;
}

beforeEach(() => {
  wsUrl = null;
  sendSpy = null;
  socketsCreated = 0;
  vi.useFakeTimers();
  vi.stubGlobal("WebSocket", MockWebSocket);
});

afterEach(() => {
  vi.clearAllTimers();
  vi.useRealTimers();
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

  function connectedClient(
    onEnvelope: (env: unknown) => void,
    onError: (e: string) => void,
  ) {
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

describe("AgentWsClient liveness", () => {
  function clientWith(onError: (error: string) => void = () => {}) {
    return new AgentWsClient({
      url: "ws://127.0.0.1:7483/ws",
      onEnvelope: () => {},
      onError,
    });
  }

  it("keeps a socket that is still receiving frames", () => {
    const client = clientWith();
    const ws = openAndHello(client);
    vi.advanceTimersByTime(9_000);
    ws.onmessage?.({
      data: JSON.stringify({ jsonrpc: "2.0", method: "server/stats", params: {} }),
    });
    vi.advanceTimersByTime(9_000);
    client.ensureLive(true);
    expect(socketsCreated).toBe(1);
  });

  it("reconnects a silent open socket once", () => {
    const client = clientWith();
    openAndHello(client);
    vi.advanceTimersByTime(10_001);
    client.ensureLive(false);
    expect(socketsCreated).toBe(2);
    client.ensureLive(false);
    expect(socketsCreated).toBe(2);
  });

  it("retries when hello does not arrive, without an auth error", () => {
    const onError = vi.fn();
    const client = clientWith(onError);
    client.connect();
    const ws = (client as unknown as { ws: MockWebSocket }).ws;
    ws.readyState = MockWebSocket.OPEN;
    ws.onopen?.();
    vi.advanceTimersByTime(2_000);
    expect(onError).not.toHaveBeenCalled();
    expect(socketsCreated).toBe(1);
    vi.advanceTimersByTime(500);
    expect(socketsCreated).toBe(2);
  });

  it("stops when the upgrade fails and only focus retries", () => {
    const onError = vi.fn();
    const client = new AgentWsClient({
      url: "ws://127.0.0.1:7483/ws",
      authToken: "tok",
      onEnvelope: () => {},
      onError,
    });
    client.connect();
    const ws = (client as unknown as { ws: MockWebSocket }).ws;
    ws.onerror?.();
    expect(onError).toHaveBeenCalledWith(
      "Authentication failed: token rejected by serve.",
    );
    vi.advanceTimersByTime(30_000);
    client.ensureLive(false);
    expect(socketsCreated).toBe(1);
    client.ensureLive(true);
    expect(socketsCreated).toBe(2);
  });

  it("skips a pending backoff when forced and waits it out otherwise", () => {
    const client = clientWith();
    const ws = openAndHello(client);
    ws.close();
    expect(socketsCreated).toBe(1);
    client.ensureLive(false);
    expect(socketsCreated).toBe(1);
    client.ensureLive(true);
    expect(socketsCreated).toBe(2);
  });

  it("keeps retrying a refused reconnect after a completed hello", () => {
    const onError = vi.fn();
    const client = new AgentWsClient({
      url: "ws://127.0.0.1:7483/ws",
      authToken: "tok",
      onEnvelope: () => {},
      onError,
    });
    const ws = openAndHello(client);
    ws.close();
    vi.advanceTimersByTime(500);
    const retry = (client as unknown as { ws: MockWebSocket }).ws;
    retry.onerror?.();
    expect(onError).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1_000);
    expect(socketsCreated).toBe(3);
  });

  it("does not revive a client torn down by disconnect", () => {
    const client = clientWith();
    openAndHello(client);
    client.disconnect();
    vi.advanceTimersByTime(30_000);
    client.ensureLive(true);
    expect(socketsCreated).toBe(1);
  });
});

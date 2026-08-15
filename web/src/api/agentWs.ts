import type { ConnectionState } from "./types";

/** JSON-RPC 2.0 wire envelope (notification or response). */
export interface WireEnvelope {
  jsonrpc?: string;
  id?: string | number;
  method?: string;
  result?: unknown;
  error?: { code: number; message: string };
  params?: Record<string, unknown>;
  [key: string]: unknown;
}

export interface AgentWsOptions {
  url?: string;
  authToken?: string;
  onEnvelope: (env: WireEnvelope) => void;
  onConnectionChange?: (state: ConnectionState) => void;
  onError?: (error: string) => void;
  reconnect?: boolean;
  reconnectBaseMs?: number;
  reconnectMaxMs?: number;
}

const DEFAULT_WS_PATH = "/ws";
const HANDSHAKE_TIMEOUT_MS = 2000;

function resolveWsUrl(explicit?: string): string {
  if (explicit) return explicit;
  if (import.meta.env.VITE_WS_URL) {
    return import.meta.env.VITE_WS_URL;
  }
  const proto = window.location.protocol === "https:" ? "wss:" : "ws:";
  return `${proto}//${window.location.host}${DEFAULT_WS_PATH}`;
}

function appendQueryParam(url: string, key: string, value: string): string {
  const sep = url.includes("?") ? "&" : "?";
  return `${url}${sep}${key}=${encodeURIComponent(value)}`;
}

export class AgentWsClient {
  private ws: WebSocket | null = null;
  private options: AgentWsOptions;
  private url: string;
  private reconnectEnabled: boolean;
  private reconnectBaseMs: number;
  private reconnectMaxMs: number;
  private reconnectAttempt = 0;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private handshakeTimer: ReturnType<typeof setTimeout> | null = null;
  private intentionalClose = false;
  private handshakeComplete = false;
  private lineBuffer = "";
  private needsAuth: boolean;

  /** In-flight JSON-RPC 2.0 promises keyed by request id. */
  private pendingRpc = new Map<
    string | number,
    { resolve: (result: unknown) => void; reject: (err: Error) => void }
  >();
  private nextRpcId = 1;

  constructor(options: AgentWsOptions) {
    this.options = options;
    this.url = resolveWsUrl(options.url);
    this.reconnectEnabled = options.reconnect ?? true;
    this.reconnectBaseMs = options.reconnectBaseMs ?? 500;
    this.reconnectMaxMs = options.reconnectMaxMs ?? 8000;
    this.needsAuth = Boolean(options.authToken);
  }

  connect(): void {
    this.intentionalClose = false;
    this.handshakeComplete = false;
    this.clearReconnectTimer();
    this.clearHandshakeTimer();
    this.setConnectionState(
      this.reconnectAttempt > 0 ? "reconnecting" : "connecting",
    );

    this.handshakeTimer = setTimeout(() => {
      if (!this.handshakeComplete) {
        this.failHandshake(this.authErrorMessage());
      }
    }, HANDSHAKE_TIMEOUT_MS);

    const wsUrl = this.buildConnectUrl();
    const ws = new WebSocket(wsUrl);
    this.ws = ws;

    ws.onopen = () => {
      this.reconnectAttempt = 0;
      this.setConnectionState("connected");
      if (this.needsAuth && this.options.authToken) {
        this.send("auth", { token: this.options.authToken });
      }
    };

    ws.onmessage = (ev) => {
      if (typeof ev.data !== "string") return;
      this.handleIncoming(ev.data);
    };

    ws.onerror = () => {
      if (!this.handshakeComplete) {
        this.failHandshake(this.authErrorMessage());
      } else {
        this.options.onError?.("WebSocket error");
      }
    };

    ws.onclose = () => {
      this.ws = null;
      this.lineBuffer = "";
      if (!this.handshakeComplete) {
        this.failHandshake(this.authErrorMessage());
        return;
      }
      if (!this.intentionalClose && this.reconnectEnabled) {
        this.scheduleReconnect();
      } else {
        this.setConnectionState("disconnected");
      }
    };
  }

  disconnect(): void {
    this.intentionalClose = true;
    this.clearReconnectTimer();
    this.clearHandshakeTimer();
    this.ws?.close();
    this.ws = null;
    this.setConnectionState("disconnected");
  }

  send(method: string, params?: Record<string, unknown>): void {
    if (!this.ws || this.ws.readyState !== WebSocket.OPEN) {
      this.options.onError?.("WebSocket not connected");
      return;
    }
    const id = this.nextRpcId++;
    const payload = JSON.stringify({
      jsonrpc: "2.0",
      id,
      method,
      params: params ?? {},
    });
    // Explicit error surface: a throw here (socket closing/closed mid-send)
    // must not be swallowed as a silent fire-and-forget (FE-10).
    try {
      this.ws.send(payload);
    } catch (error) {
      this.options.onError?.(
        `WebSocket send failed: ${error instanceof Error ? error.message : String(error)}`,
      );
    }
  }

  /** Send a JSON-RPC 2.0 request and return a Promise for the result. */
  sendJsonRpc<T = unknown>(
    method: string,
    params?: Record<string, unknown>,
  ): Promise<T> {
    return new Promise((resolve, reject) => {
      if (!this.ws || this.ws.readyState !== WebSocket.OPEN) {
        reject(new Error("WebSocket not connected"));
        return;
      }
      const id = `rpc-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
      const timeout = setTimeout(() => {
        this.pendingRpc.delete(id);
        reject(new Error(`RPC timeout: ${method}`));
      }, 30_000);

      this.pendingRpc.set(id, {
        resolve: (result: unknown) => {
          clearTimeout(timeout);
          resolve(result as T);
        },
        reject: (err: Error) => {
          clearTimeout(timeout);
          reject(err);
        },
      });

      const payload = JSON.stringify({
        jsonrpc: "2.0",
        id,
        method,
        params: params ?? {},
      });
      this.ws.send(payload);
    });
  }

  isConnected(): boolean {
    return this.ws?.readyState === WebSocket.OPEN;
  }

  private buildConnectUrl(): string {
    let url = this.url;
    const token = this.options.authToken;
    if (token) {
      // url already contains any caller-supplied query params (e.g. session);
      // append the token param preserving existing query string.
      url = appendQueryParam(url, "token", token);
    }
    return url;
  }

  private authErrorMessage(): string {
    if (this.options.authToken) {
      return "Authentication failed: token rejected by serve.";
    }
    return "Authentication required: host must inject an auth token (dev: VITE_AUTH_TOKEN / LITECODE_TOKEN).";
  }

  private failHandshake(message: string): void {
    if (this.intentionalClose) return;
    this.intentionalClose = true;
    this.clearHandshakeTimer();
    this.clearReconnectTimer();
    this.ws?.close();
    this.ws = null;
    this.setConnectionState("disconnected");
    this.options.onError?.(message);
  }

  private handleIncoming(chunk: string): void {
    this.lineBuffer += chunk;
    const lines = this.lineBuffer.split("\n");
    this.lineBuffer = lines.pop() ?? "";

    for (const line of lines) {
      this.dispatchLine(line);
    }

    if (this.lineBuffer.trim()) {
      const pending = this.lineBuffer;
      this.lineBuffer = "";
      this.dispatchLine(pending);
    }
  }

  private dispatchLine(line: string): void {
    const trimmed = line.trim();
    if (!trimmed) return;
    try {
      const json: unknown = JSON.parse(trimmed);
      if (!json || typeof json !== "object") {
        this.options.onError?.("Unrecognized response shape");
        return;
      }

      // ── JSON-RPC 2.0 ──
      if ("jsonrpc" in json) {
        const rpc = json as {
          id?: string | number;
          method?: string;
          result?: unknown;
          error?: { code: number; message: string };
          params?: Record<string, unknown>;
        };
        const id = rpc.id;

        // RPC Response (has id): resolve pending promise
        if (id != null) {
          if (this.pendingRpc.has(id)) {
            const { resolve, reject } = this.pendingRpc.get(id)!;
            this.pendingRpc.delete(id);
            if ("result" in rpc) {
              resolve(rpc.result);
            } else if (rpc.error) {
              reject(new Error(rpc.error.message));
            } else {
              reject(new Error("Invalid JSON-RPC response"));
            }
            return;
          }
          // RPC response with no pending handler — pass through as envelope
          // (e.g. lsp/request responses are handled by litecodeLsp.ts)
          if (rpc.method === undefined) {
            this.options.onEnvelope(json as WireEnvelope);
            return;
          }
        }

        // Notification (has method, no id): pass to onEnvelope
        if (rpc.method !== undefined && id == null) {
          if (rpc.method === "server/hello") {
            this.handshakeComplete = true;
            this.clearHandshakeTimer();
          }
          this.options.onEnvelope(json as WireEnvelope);
          return;
        }

        // Unknown JSON-RPC message — ignore
        return;
      }

      this.options.onEnvelope(json as WireEnvelope);
    } catch {
      this.options.onError?.(`Invalid JSON: ${trimmed.slice(0, 80)}`);
    }
  }

  private scheduleReconnect(): void {
    this.setConnectionState("reconnecting");
    const delay = Math.min(
      this.reconnectBaseMs * 2 ** this.reconnectAttempt,
      this.reconnectMaxMs,
    );
    this.reconnectAttempt += 1;
    this.reconnectTimer = setTimeout(() => this.connect(), delay);
  }

  private clearReconnectTimer(): void {
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
  }

  private clearHandshakeTimer(): void {
    if (this.handshakeTimer) {
      clearTimeout(this.handshakeTimer);
      this.handshakeTimer = null;
    }
  }

  private setConnectionState(state: ConnectionState): void {
    this.options.onConnectionChange?.(state);
  }
}

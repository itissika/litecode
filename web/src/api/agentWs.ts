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
/** Server pushes `server/stats` every 2s. Silence longer than this means the
 *  socket is open in name only (sleep, half-open TCP). */
const STALE_AFTER_MS = 10_000;
/** Drop a stuck partial frame rather than holding it forever. */
const MAX_WS_BUFFER = 8 * 1024 * 1024;

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
  /** App teardown. Unlike an auth give-up, focus must not revive this client. */
  private stopped = false;
  private handshakeComplete = false;
  /** A completed hello means a later refused upgrade is a down server, not a
   *  bad token. First-connect rejection still stops. */
  private hadHello = false;
  /** True once this attempt's `onopen` has run. Closing before that, without
   *  the hello timer, is an upgrade rejection. A hello timeout keeps retrying. */
  private socketOpened = false;
  /** Hello timer fired. `onclose` from that close must keep retrying. */
  private handshakeTimedOut = false;
  /** Bumped on each `connect` so a replaced socket cannot schedule another. */
  private generation = 0;
  private connectStartedAt = 0;
  private lastInboundAt = 0;
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
    if (this.stopped) return;
    this.intentionalClose = false;
    this.handshakeComplete = false;
    this.socketOpened = false;
    this.handshakeTimedOut = false;
    this.lineBuffer = "";
    this.clearReconnectTimer();
    this.clearHandshakeTimer();
    const previous = this.ws;
    this.ws = null;
    this.generation += 1;
    const generation = this.generation;
    previous?.close();

    this.connectStartedAt = Date.now();
    this.setConnectionState(
      this.reconnectAttempt > 0 ? "reconnecting" : "connecting",
    );

    this.handshakeTimer = setTimeout(() => {
      if (
        generation !== this.generation ||
        this.handshakeComplete ||
        this.intentionalClose ||
        this.stopped
      ) {
        return;
      }
      // The upgrade was accepted (or is still in flight) but hello never
      // arrived. Keep the backoff loop; this is not a rejected token.
      this.handshakeTimedOut = true;
      this.ws?.close();
      this.scheduleReconnect();
    }, HANDSHAKE_TIMEOUT_MS);

    const wsUrl = this.buildConnectUrl();
    const ws = new WebSocket(wsUrl);
    this.ws = ws;
    const current = () => generation === this.generation;

    ws.onopen = () => {
      if (!current()) return;
      this.socketOpened = true;
      this.reconnectAttempt = 0;
      this.setConnectionState("connected");
      if (this.needsAuth && this.options.authToken) {
        this.send("auth", { token: this.options.authToken });
      }
    };

    ws.onmessage = (ev) => {
      if (!current()) return;
      if (typeof ev.data !== "string") return;
      this.lastInboundAt = Date.now();
      this.handleIncoming(ev.data);
    };

    ws.onerror = () => {
      if (!current()) return;
      // Upgrade failed before the socket opened: bad token or refused.
      // A later close of an already-open socket is handled by `onclose`.
      if (!this.handshakeComplete && !this.socketOpened) {
        if (this.hadHello) {
          this.ws?.close();
        } else {
          this.failHandshake(this.authErrorMessage());
        }
      } else if (this.handshakeComplete) {
        this.options.onError?.("WebSocket error");
      }
    };

    ws.onclose = () => {
      if (!current()) return;
      this.ws = null;
      this.lineBuffer = "";
      if (this.stopped || this.intentionalClose) return;
      if (this.handshakeTimedOut) {
        this.handshakeTimedOut = false;
        this.scheduleReconnect();
        return;
      }
      if (!this.handshakeComplete && !this.socketOpened) {
        if (this.hadHello && this.reconnectEnabled) {
          this.scheduleReconnect();
        } else {
          this.failHandshake(this.authErrorMessage());
        }
        return;
      }
      if (this.reconnectEnabled) {
        this.scheduleReconnect();
      } else {
        this.setConnectionState("disconnected");
      }
    };
  }

  /**
   * Reconnect when this socket is gone or has gone silent.
   * `force` is the window-focus path: it also retries once after an auth
   * give-up, and it skips a pending backoff. A healthy socket stays up.
   */
  ensureLive(force = false): void {
    if (this.stopped) return;
    if (this.intentionalClose) {
      if (force) this.connect();
      return;
    }

    const ws = this.ws;
    const open = ws?.readyState === WebSocket.OPEN;
    if (open && this.handshakeComplete) {
      if (
        this.lastInboundAt > 0 &&
        Date.now() - this.lastInboundAt > STALE_AFTER_MS
      ) {
        this.reconnectAttempt = Math.max(this.reconnectAttempt, 1);
        this.connect();
      }
      return;
    }

    if (this.reconnectTimer) {
      if (!force) return;
      this.clearReconnectTimer();
      this.connect();
      return;
    }

    if (!ws) {
      if (force) this.connect();
      return;
    }

    if (
      force &&
      Date.now() - this.connectStartedAt >= HANDSHAKE_TIMEOUT_MS
    ) {
      this.connect();
    }
  }

  disconnect(): void {
    this.stopped = true;
    this.intentionalClose = true;
    this.clearReconnectTimer();
    this.clearHandshakeTimer();
    const ws = this.ws;
    this.ws = null;
    this.generation += 1;
    ws?.close();
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
    if (this.lineBuffer.length > MAX_WS_BUFFER) {
      const preview = this.lineBuffer.trimStart().slice(0, 80);
      this.lineBuffer = "";
      this.options.onError?.(`Invalid JSON: ${preview}`);
      return;
    }
    const lines = this.lineBuffer.split("\n");
    this.lineBuffer = lines.pop() ?? "";

    for (const line of lines) {
      this.dispatchLine(line);
    }

    // One WS text frame is usually one compact JSON object with no trailing
    // newline. Parse the remainder only when it is complete; otherwise keep it
    // so a split frame is not toasted as Invalid JSON.
    const pending = this.lineBuffer.trim();
    if (!pending) return;
    try {
      JSON.parse(pending);
    } catch {
      return;
    }
    this.lineBuffer = "";
    this.dispatchLine(pending);
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
            this.hadHello = true;
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
    if (this.stopped || this.intentionalClose || !this.reconnectEnabled) {
      if (this.stopped || !this.reconnectEnabled) {
        this.setConnectionState("disconnected");
      }
      return;
    }
    if (this.reconnectTimer) return;
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

#!/usr/bin/env node
/**
 * Manual WS smoke probe against `litecode serve` (JSON-RPC 2.0 over WebSocket).
 * The token rides the handshake URL (`?token=`); no auth frame is sent.
 * Usage:
 *   node web/scripts/ws-smoke.mjs handshake [--token secret]   # expect server/hello
 *   node web/scripts/ws-smoke.mjs list [--token secret]        # also call session/list
 *   node web/scripts/ws-smoke.mjs bad-handshake [--bad-token]  # wrong token: no server/hello
 */
const WebSocket = globalThis.WebSocket;
if (!WebSocket) {
  console.error("WebSocket unavailable; need Node 22+");
  process.exit(2);
}

const BASE = process.env.WS_URL ?? "ws://127.0.0.1:7483/ws";
const mode = process.argv[2] ?? "handshake";
const token =
  process.argv.includes("--bad-token") || mode === "bad-handshake"
    ? "wrong"
    : (process.argv.find((_, i, a) => a[i - 1] === "--token") ?? process.env.WS_TOKEN ?? "secret");
const timeoutMs = Number(process.env.WS_TIMEOUT_MS ?? 2500);
const LIST_ID = 1;

function wsUrl() {
  const sep = BASE.includes("?") ? "&" : "?";
  return `${BASE}${sep}token=${encodeURIComponent(token)}`;
}

/** Frames may carry several newline-delimited JSON-RPC messages. */
function ingestFrame(data, state) {
  for (const line of String(data).split("\n")) {
    const text = line.trim();
    if (!text) continue;
    try {
      state.messages.push(JSON.parse(text));
    } catch (e) {
      state.parseErrors.push({ line: text.slice(0, 120), error: String(e) });
    }
  }
}

const hasHello = (state) => state.messages.some((m) => m?.method === "server/hello");
const listResponse = (state) => state.messages.find((m) => m?.id === LIST_ID && ("result" in m || "error" in m));

function runScenario(name, { callList = false } = {}) {
  return new Promise((resolve) => {
    const state = { messages: [], parseErrors: [], closed: null, opened: false, listSent: false };
    const url = wsUrl();
    const ws = new WebSocket(url);
    const started = Date.now();
    let done = false;

    const timer = setTimeout(() => {
      ws.close();
      finish("timeout");
    }, timeoutMs);

    function finish(reason) {
      if (done) return;
      done = true;
      clearTimeout(timer);
      const list = listResponse(state);
      resolve({
        name,
        reason,
        url,
        opened: state.opened,
        closed: state.closed,
        hasServerHello: hasHello(state),
        sessionListOk: Boolean(list && Array.isArray(list.result?.sessions)),
        sessionListError: list?.error ?? null,
        messageCount: state.messages.length,
        parseErrors: state.parseErrors,
        elapsedMs: Date.now() - started,
      });
    }

    ws.addEventListener("open", () => {
      state.opened = true;
    });

    ws.addEventListener("message", (ev) => {
      ingestFrame(ev.data, state);
      if (!hasHello(state)) return;
      if (!callList) {
        ws.close();
        finish("server/hello");
        return;
      }
      if (!state.listSent) {
        state.listSent = true;
        ws.send(JSON.stringify({ jsonrpc: "2.0", id: LIST_ID, method: "session/list", params: {} }));
        return;
      }
      if (listResponse(state)) {
        ws.close();
        finish("session/list");
      }
    });

    ws.addEventListener("close", (ev) => {
      state.closed = { code: ev.code, reason: ev.reason };
      finish("close");
    });

    ws.addEventListener("error", () => {
      state.closed = { error: "websocket error" };
      finish("error");
    });
  });
}

let result;
if (mode === "bad-handshake") {
  result = await runScenario("bad-token-handshake");
} else if (mode === "list") {
  result = await runScenario("session_list", { callList: true });
} else {
  result = await runScenario("handshake");
}

console.log(JSON.stringify(result, null, 2));
const pass =
  mode === "bad-handshake" ? !result.hasServerHello : mode === "list" ? result.sessionListOk : result.hasServerHello;
process.exit(pass ? 0 : 1);

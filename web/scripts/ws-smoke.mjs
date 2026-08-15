#!/usr/bin/env node
/**
 * Temporary WS smoke probe (Phase 0 manual acceptance automation).
 * Usage:
 *   node web/scripts/ws-smoke.mjs handshake [--token secret]
 *   node web/scripts/ws-smoke.mjs list
 *   node web/scripts/ws-smoke.mjs bad-handshake [--bad-token]
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

function wsUrl() {
  const sep = BASE.includes("?") ? "&" : "?";
  return `${BASE}${sep}token=${encodeURIComponent(token)}`;
}

function ingestFrame(data, state) {
  const text = String(data).trim();
  if (!text) return;
  try {
    state.envelopes.push(JSON.parse(text));
  } catch (e) {
    state.parseErrors.push({ line: text.slice(0, 120), error: String(e) });
  }
}

function hasKey(envelopes, key) {
  return envelopes.some((e) => e && typeof e === "object" && key in e);
}

function runScenario(name, { sendAuth = true, sendListSessions = false } = {}) {
  return new Promise((resolve) => {
    const state = { buf: "", envelopes: [], parseErrors: [], closed: null, opened: false };
    const url = wsUrl();
    const ws = new WebSocket(url);
    let done = false;

    const timer = setTimeout(() => {
      if (!done) {
        done = true;
        ws.close();
        finish("timeout");
      }
    }, timeoutMs);

    function finish(reason) {
      if (done && reason !== "timeout") return;
      if (!done) done = true;
      clearTimeout(timer);
      resolve({
        name,
        reason,
        url,
        token,
        opened: state.opened,
        closed: state.closed,
        envelopes: state.envelopes,
        parseErrors: state.parseErrors,
        hasServerHello: hasKey(state.envelopes, "server_hello"),
        hasSessionSnapshot: hasKey(state.envelopes, "session_snapshot"),
        hasSessionList: hasKey(state.envelopes, "session_list"),
        elapsedMs: Date.now() - started,
      });
    }

    const started = Date.now();

    ws.addEventListener("open", () => {
      state.opened = true;
      if (sendAuth) {
        ws.send(JSON.stringify({ auth: { token } }));
      }
      if (sendListSessions) {
        setTimeout(() => ws.send(JSON.stringify({ list_sessions: null })), 50);
      }
    });

    ws.addEventListener("message", (ev) => {
      ingestFrame(ev.data, state);
      if (sendListSessions && hasKey(state.envelopes, "session_list")) {
        ws.close();
        finish("session_list");
        return;
      }
      if (
        !sendListSessions &&
        hasKey(state.envelopes, "server_hello") &&
        hasKey(state.envelopes, "session_snapshot")
      ) {
        ws.close();
        finish("handshake");
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
  result = await runScenario("bad-token-handshake", { sendAuth: true });
} else if (mode === "list") {
  result = await runScenario("list_sessions", { sendAuth: true, sendListSessions: true });
} else {
  result = await runScenario("handshake", { sendAuth: true });
}

console.log(JSON.stringify(result, null, 2));
process.exit(
  mode === "bad-handshake"
    ? result.hasServerHello || result.hasSessionSnapshot
      ? 1
      : 0
    : mode === "list"
      ? result.hasSessionList
        ? 0
        : 1
      : result.hasServerHello && result.hasSessionSnapshot
        ? 0
        : 1,
);

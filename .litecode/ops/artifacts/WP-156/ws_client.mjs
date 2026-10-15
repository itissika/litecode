// WP-156 simulated frontend open sequence (frontend_mid, HEAD 3ddc643; cross-checked with web/ source):
//   T0 = new WebSocket(/ws?token=)                    web/src/api/agentWs.ts:130-131, :327-336
//   on open, no waiting, in order:
//     session/list                                    web/src/stores/connectionStore.ts:157-164
//     session/subscribe (only if a panel is restored) web/src/dockview/panels/AgentPanel.tsx:112-117
//     auth{token} (only if token)                     web/src/api/agentWs.ts:136-141
//   on server/hello (server push; client never sends hello; 2000ms hello timeout -> frontend reconnects):
//     GET /api/workspace/engines, GET /api/settings, session/new (concurrent)
//   on session/new reply: session/subscribe(new id)   web/src/stores/sessionStore.ts:591-595
//   on session/snapshot: buffer/load(next_seq-40..)   web/src/stores/sessionStore.ts:216-229 (recorded, not an unlock)
// Paths: A = new profile, unlock = session/new reply; B = restored layout,
//   unlock = session/list reply containing the restored session. Snapshot times recorded for reference.
// Records type/time/bytes only; never content. Session ids hashed (sha256/8).
// usage: TOKEN=... node ws_client.mjs 127.0.0.1:PORT A|B [restored_sid] [grace_ms]
import { createRequire } from "node:module";
import { createHash } from "node:crypto";
import http from "node:http";
const require = createRequire(import.meta.url);
const WebSocket = require(process.env.WS_MODULE || "/workspace/litecode/web/node_modules/ws");
const [base, path, sidArg, graceArg] = process.argv.slice(2);
const grace = Number(graceArg ?? 1500);
const token = process.env.TOKEN || "";
const h = (s) => (s ? createHash("sha256").update(s).digest("hex").slice(0, 8) : "");
const t0 = performance.now(); const epoch0 = Date.now();
const rel = () => +(performance.now() - t0).toFixed(1);
const ev = []; const marks = {};
const mark = (k) => { if (marks[k] == null) marks[k] = rel(); };
let nextId = 1; const pending = new Map();
const restored = path === "B" ? sidArg : null;
const ws = new WebSocket(`ws://${base}/ws` + (token ? `?token=${token}` : ""));
const send = (method, params) => { const id = nextId++; pending.set(id, method); ws.send(JSON.stringify({ jsonrpc: "2.0", id, method, ...(params ? { params } : {}) })); ev.push({ t: rel(), dir: "out", m: method }); };
const get = (p) => {
  ev.push({ t: rel(), dir: "out", m: "GET " + p });
  const [host, port] = base.split(":");
  const req = http.get({ host, port, path: p, headers: token ? { Authorization: `Bearer ${token}` } : {} }, (res) => {
    let n = 0; res.on("data", (c) => (n += c.length));
    res.on("end", () => { ev.push({ t: rel(), dir: "in", m: "GET " + p, status: res.statusCode, bytes: n }); mark("GET " + p); check(); });
  });
  req.on("error", (e) => ev.push({ t: rel(), m: "GET " + p + " error " + e.code }));
};
const subscribed = new Set();
const subscribe = (sid) => { if (sid && !subscribed.has(sid)) { subscribed.add(sid); send("session/subscribe", { session_id: sid }); } };
const loaded = new Set();
let finishing = false;
const check = () => {
  const need = ["reply:session/list", "reply:session/new", "GET /api/workspace/engines", "GET /api/settings"];
  if (!finishing && need.every((k) => marks[k] != null)) { finishing = true; setTimeout(() => ws.close(), grace); }
};
ws.on("open", () => { mark("ws_open"); ev.push({ t: rel(), dir: "in", m: "ws_open" }); send("session/list"); subscribe(restored); if (token) send("auth", { token }); });
ws.on("message", (data) => {
  const t = rel();
  for (const line of data.toString().split("\n")) {
    if (!line.trim()) continue; let j; try { j = JSON.parse(line); } catch { continue; }
    const e = { t, dir: "in", bytes: line.length };
    if (j.method) e.m = j.method; else if (j.id != null) { e.m = "reply:" + (pending.get(j.id) ?? j.id); if (j.error) e.err = j.error.code; } else e.m = j.type ?? "envelope";
    if (j.method === "server/hello") { mark("server/hello"); if (t > 2000) e.late_hello = true; get("/api/workspace/engines"); get("/api/settings"); send("session/new"); }
    if (j.method === "session/snapshot") {
      const p = j.params ?? {}; e.sid = h(p.session_id); const ph = p.turn?.phase;
      e.phase = !p.turn ? "no_turn" : typeof ph === "object" && ph ? (ph.kind ?? Object.keys(ph)[0]) : ph ?? null;
      mark("snapshot:" + e.sid);
      if (p.session_id && !loaded.has(p.session_id)) { loaded.add(p.session_id); const to = p.buffer?.next_seq ?? 0; send("buffer/load", { from_seq: Math.max(0, to - 40), to_seq: to, session_id: p.session_id }); }
    }
    if (e.m === "reply:session/list") { const ss = j.result?.sessions ?? []; e.sessions = ss.length; if (restored) { e.has_restored = ss.some((s) => s.id === restored); if (e.has_restored) mark("unlock_B"); } }
    if (e.m === "reply:session/new") { const sid = j.result?.session_id; e.sid = h(sid); mark("unlock_A"); subscribe(sid); }
    if (e.m === "reply:buffer/load") e.events = j.result?.events?.length;
    mark(e.m); ev.push(e);
  }
  check();
});
const hard = setTimeout(() => { ev.push({ t: rel(), m: "timeout" }); ws.close(); }, 60000);
ws.on("close", () => { clearTimeout(hard); console.log(JSON.stringify({ epoch0, path, restored: h(restored), unlock: path === "A" ? marks.unlock_A : marks.unlock_B, marks, events: ev })); process.exit(0); });
ws.on("error", (e) => ev.push({ t: rel(), m: "error:" + e.code }));

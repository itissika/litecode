// Read-only probe against the running backend (127.0.0.1:7483):
//  1. session/list  -> find the running parent session
//  2. session/subscribe -> dump the snapshot's meter fields
//  3. listen 8s for live turn events on that session
// Never mutates anything; closes the socket at the end.
const URL = "ws://127.0.0.1:7483/ws?token=OVLB1hYzw1ZYNdhqq9VZnBsyPaWcj1MzwWV6br3iejY";
const ws = new WebSocket(URL);
let nextId = 1;
const pending = new Map();
const send = (method, params) => {
  const id = nextId++;
  ws.send(JSON.stringify({ jsonrpc: "2.0", id, method, params: params ?? {} }));
  return new Promise((res, rej) => {
    const t = setTimeout(() => { pending.delete(id); rej(new Error("timeout " + method)); }, 15000);
    pending.set(id, { res: (v) => { clearTimeout(t); res(v); }, rej: (e) => { clearTimeout(t); rej(new Error(e)); } });
  });
};
const seen = [];
ws.onmessage = (ev) => {
  let m; try { m = JSON.parse(ev.data); } catch { return; }
  if (m.id !== undefined && pending.has(m.id)) {
    const p = pending.get(m.id); pending.delete(m.id);
    m.error ? p.rej(m.error.message) : p.res(m.result);
    return;
  }
  if (m.method) {
    seen.push({ method: m.method, params: m.params });
    if (["session/attached", "session/snapshot", "agent/turn_started", "agent/turn_finished"].includes(m.method) || seen.length < 40) {
      frames.push(m);
    }
  }
};
const frames = [];
ws.onerror = (e) => { console.error("WS error", e.message ?? e); process.exit(1); };
ws.onclose = () => console.log("socket closed");

ws.onopen = async () => {
  try {
    const list = await send("session/list");
    const sessions = list?.sessions ?? list ?? [];
    const rows = sessions.map(s => ({ id: s.id.slice(0, 12), agent: s.agent_id, running: s.running, status: s.status, parent: s.parent_session_id?.slice(0, 8) ?? null, ctx: s.turn?.context_window ?? null }));
    console.log("== session/list (" + rows.length + ") ==");
    for (const r of rows) console.log(JSON.stringify(r));
    const target = sessions.find(s => s.running && !s.parent_session_id) ?? sessions.find(s => !s.parent_session_id);
    if (!target) { console.log("no target session"); ws.close(); return; }
    console.log("\n== subscribing " + target.id + " ==");
    await send("session/subscribe", { session_id: target.id });
    await new Promise(r => setTimeout(r, 4000));
    // dump captured frames for offline replay
    const fs = await import("node:fs");
    fs.writeFileSync("probe-frames.json", JSON.stringify(frames, null, 1));
    console.log("frames captured:", frames.length, "-> probe-frames.json");
    const snapF = frames.find(f => f.method === "session/snapshot");
    if (snapF) {
      const p = snapF.params;
      console.log("snapshot meter:", JSON.stringify({
        context_window: p.context_window,
        context_tokens_estimate: p.context_tokens_estimate,
        last_turn: p.last_turn_token_stats,
        turn_phase: p.turn?.phase,
        turn_step: p.turn?.step,
      }));
    }
    const attF = frames.find(f => f.method === "session/attached");
    if (attF) console.log("attached turn phase:", attF.params?.turn?.phase);
    ws.close();
    setTimeout(() => process.exit(0), 300);
    return;
  } catch (e) {
    console.error("probe failed:", e.message ?? e);
  } finally {
    ws.close();
    setTimeout(() => process.exit(0), 300);
  }
};

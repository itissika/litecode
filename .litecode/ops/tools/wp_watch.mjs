// ops.wp_watch (v11) — resident WP-timeout watcher with callback delivery.
//   start | stop | status | supervise | run | once     (see tools/board.md「ops.wp_watch 用法」)
// Env (never written to repo / git config):
//   LITECODE_WP_WEBHOOK_URL  callback URL (POST JSON)
//   LITECODE_WP_WEBHOOK_KEY  secret → Authorization: Bearer <key> + X-LiteCode-Signature HMAC-SHA256
import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import {
  loadRegistry, printVersionHeader, opsRoot, recordTrace, scanAndEmitWpTimeouts,
  watchPaths, ensureTmpDir, writeJsonAtomic, readJsonOr, isWatchPidAlive,
} from "./_lib.mjs";

const TOOL = "ops.wp_watch";
const SELF = fileURLToPath(import.meta.url);
const { reg } = loadRegistry(import.meta.url);
printVersionHeader(reg, TOOL);
const ops = opsRoot(import.meta.url);
const P = watchPaths(ops);
ensureTmpDir(ops);

const argv = process.argv.slice(2);
const cmd = argv[0] && !argv[0].startsWith("--") ? argv[0] : "status";
const rest = cmd === argv[0] ? argv.slice(1) : argv;
function num(name, def) { const i = rest.indexOf(name); const v = i >= 0 ? Number(rest[i + 1]) : NaN; return Number.isFinite(v) && v > 0 ? v : def; }
function opt(name) { const i = rest.indexOf(name); return i >= 0 ? rest[i + 1] : null; }
const INTERVAL_MS = num("--interval-sec", 300) * 1000;
const HB_MS = Math.min(num("--heartbeat-sec", 60) * 1000, INTERVAL_MS);
const FORCE = rest.includes("--force-timeout-scan");
const wpTrace = opt("--wp");
const LOG_MAX = 1024 * 1024; // rotate at 1 MiB → wp_watch.log.1 (one generation kept)
const ROLE = cmd === "supervise" ? "sup" : cmd === "run" || cmd === "once" ? "worker" : "ctl";

function log(level, msg) {
  const line = `${new Date().toISOString()} [${ROLE} ${process.pid}] ${level} ${msg}\n`;
  try {
    try { if (fs.statSync(P.log).size > LOG_MAX) fs.renameSync(P.log, P.log + ".1"); } catch {}
    fs.appendFileSync(P.log, line);
  } catch {}
  if (ROLE === "ctl" || cmd === "once") process.stderr.write(line);
}
function readPid() { const s = readJsonOr(P.pid, null); return s && Number.isInteger(s.pid) ? s : null; }
const rel = (p) => path.relative(path.join(ops, "..", ".."), p);

// ── worker ──────────────────────────────────────────────────────────────────
let lastCycleAt = null, lastError = null, cycleNo = 0;
function heartbeat(extra = {}) {
  try {
    writeJsonAtomic(P.heartbeat, {
      pid: process.pid,
      supervisor_pid: Number(process.env.LITECODE_WP_WATCH_SUPERVISOR) || null,
      ts: new Date().toISOString(),
      last_cycle_at: lastCycleAt,
      last_error: lastError,
      cycles: cycleNo,
      interval_sec: INTERVAL_MS / 1000,
      tools_version: reg.tools_version,
      ...extra,
    });
  } catch (e) { log("ERROR", "heartbeat write failed: " + e.message); }
}
function loadState() {
  const s = readJsonOr(P.state, null) || {};
  return { delivered: s.delivered || {}, retry: s.retry || {}, updated_at: s.updated_at || null };
}
function saveState(s) { s.updated_at = new Date().toISOString(); writeJsonAtomic(P.state, s); }
function payloadOf(ev) {
  const m = ev.meta || {};
  return {
    event_id: ev.id, wp_id: ev.wp, status: m.status ?? null, age_minutes: m.age_minutes ?? null,
    owner: m.owner ?? null, reviewer: m.reviewer ?? null, status_entered_at: m.status_entered_at ?? null,
    summary: ev.summary, ts: ev.ts,
  };
}
async function post(url, key, payload) {
  const body = JSON.stringify(payload);
  const t = String(Math.floor(Date.now() / 1000));
  const sig = crypto.createHmac("sha256", key).update(`${t}.${body}`).digest("hex");
  const r = await fetch(url, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      authorization: `Bearer ${key}`,
      "x-litecode-event-id": payload.event_id,
      "x-litecode-timestamp": t,
      "x-litecode-signature": `sha256=${sig}`,
      "user-agent": `litecode-ops-wp_watch/${reg.tools_version}`,
    },
    body,
    signal: AbortSignal.timeout(10000),
  });
  try { await r.arrayBuffer(); } catch {}
  return r.status;
}
async function cycle() {
  cycleNo++;
  const now = new Date();
  const scan = scanAndEmitWpTimeouts(ops, { force: FORCE, now, source: "ops.wp_watch" });
  const box = JSON.parse(fs.readFileSync(path.join(ops, "events", "inbox.json"), "utf8"));
  const state = loadState();
  const pending = box.inbox.filter((e) => e.type === "team_ops.wp_timeout" && (!e.acked || e.acked.length === 0) && !state.delivered[e.id]);
  const url = process.env.LITECODE_WP_WEBHOOK_URL, key = process.env.LITECODE_WP_WEBHOOK_KEY;
  let sent = 0, failed = 0, deferred = 0;
  if (pending.length && (!url || !key)) {
    log("WARN", `delivery skipped: ${!url ? "LITECODE_WP_WEBHOOK_URL" : ""}${!url && !key ? "+" : ""}${!key ? "LITECODE_WP_WEBHOOK_KEY" : ""} unset; ${pending.length} undelivered wp_timeout event(s): ${pending.map((e) => e.id).join(",")}`);
  } else if (pending.length) {
    for (const ev of pending) {
      const r = state.retry[ev.id];
      if (r && Date.parse(r.next_at) > Date.now()) { deferred++; continue; }
      let code = null, err = null;
      try { code = await post(url, key, payloadOf(ev)); } catch (e) { err = e.name === "TimeoutError" ? "timeout" : (e.cause?.code || e.message); }
      if (code !== null && code >= 200 && code < 300) {
        state.delivered[ev.id] = { at: new Date().toISOString(), http_status: code, wp: ev.wp, attempts: (r?.attempts || 0) + 1 };
        delete state.retry[ev.id];
        sent++;
        log("INFO", `delivered ${ev.id} (${ev.wp}) http=${code}`);
      } else {
        const attempts = (r?.attempts || 0) + 1;
        const backoffMs = Math.min(INTERVAL_MS * 2 ** (attempts - 1), 60 * 60 * 1000);
        state.retry[ev.id] = { attempts, last_error: err ?? `http ${code}`, last_try_at: new Date().toISOString(), next_at: new Date(Date.now() + backoffMs).toISOString() };
        failed++;
        log("WARN", `delivery failed ${ev.id} (${ev.wp}) ${err ?? "http " + code}; attempt ${attempts}, retry after ${Math.round(backoffMs / 1000)}s`);
      }
    }
    saveState(state);
  }
  // drop retry entries for events that got acked meanwhile
  for (const id of Object.keys(state.retry)) {
    const e = box.inbox.find((x) => x.id === id);
    if (!e || (e.acked && e.acked.length)) { delete state.retry[id]; saveState(state); }
  }
  lastCycleAt = new Date().toISOString();
  lastError = null;
  const summary = `cycle#${cycleNo} work_hours=${scan.work_hours} stale=${scan.stale.length} emitted=${scan.emitted.length} pending=${pending.length} sent=${sent} failed=${failed} deferred=${deferred}`;
  log("INFO", summary);
  heartbeat();
  return { scan, pending: pending.length, sent, failed, deferred };
}
async function runWorker(loop) {
  process.on("unhandledRejection", (e) => { lastError = "unhandledRejection: " + (e?.stack || e); log("FATAL", lastError); heartbeat(); process.exit(70); });
  process.on("uncaughtException", (e) => { lastError = "uncaughtException: " + (e?.stack || e); log("FATAL", lastError); heartbeat(); process.exit(70); });
  for (const s of ["SIGTERM", "SIGINT"]) process.on(s, () => { log("INFO", `worker ${s}, exiting`); process.exit(0); });
  log("INFO", `worker start interval=${INTERVAL_MS / 1000}s heartbeat=${HB_MS / 1000}s force_scan=${FORCE} webhook=${process.env.LITECODE_WP_WEBHOOK_URL ? "configured" : "unset"} key=${process.env.LITECODE_WP_WEBHOOK_KEY ? "configured" : "unset"} tools_version=${reg.tools_version}`);
  recordTrace(ops, wpTrace, TOOL, reg.tools_version);
  heartbeat();
  if (!loop) { const r = await cycle(); console.log(JSON.stringify({ ok: true, mode: "once", ...r, scan: { work_hours: r.scan.work_hours, stale: r.scan.stale.length, emitted: r.scan.emitted.map((e) => e.id) } })); return; }
  let running = false;
  const tick = async () => {
    if (running) return; running = true;
    try { await cycle(); } catch (e) { lastError = String(e?.stack || e); log("ERROR", "cycle failed: " + lastError); heartbeat(); }
    finally { running = false; }
  };
  setInterval(() => heartbeat(), HB_MS);
  setInterval(tick, INTERVAL_MS);
  await tick();
}

// ── supervisor ──────────────────────────────────────────────────────────────
function runSupervisor() {
  const cur = readPid();
  if (cur && cur.pid !== process.pid && isWatchPidAlive(cur.pid)) { log("ERROR", `already running pid=${cur.pid}; refusing double start`); process.exit(3); }
  writeJsonAtomic(P.pid, { pid: process.pid, started_at: new Date().toISOString(), args: rest, tools_version: reg.tools_version });
  let child = null, stopping = false, backoff = 1000, startedAt = 0, restarts = 0;
  const spawnWorker = () => {
    startedAt = Date.now();
    child = spawn(process.execPath, [SELF, "run", ...rest], { stdio: "ignore", env: { ...process.env, LITECODE_WP_WATCH_SUPERVISOR: String(process.pid) } });
    log("INFO", `spawned worker pid=${child.pid} (restarts=${restarts})`);
    child.on("exit", (code, sig) => {
      if (stopping) return;
      if (Date.now() - startedAt > 60000) backoff = 1000;
      restarts++;
      log("WARN", `worker pid=${child.pid} exited code=${code} sig=${sig}; restart in ${backoff / 1000}s`);
      setTimeout(spawnWorker, backoff);
      backoff = Math.min(backoff * 2, 60000);
    });
  };
  const stop = (s) => {
    if (stopping) return; stopping = true;
    log("INFO", `supervisor ${s}, stopping worker`);
    const done = () => { try { const c = readPid(); if (c && c.pid === process.pid) fs.unlinkSync(P.pid); } catch {} log("INFO", "supervisor stopped"); process.exit(0); };
    if (child && child.exitCode === null && child.signalCode === null) { child.once("exit", done); child.kill("SIGTERM"); setTimeout(() => { try { child.kill("SIGKILL"); } catch {} done(); }, 5000); }
    else done();
  };
  for (const s of ["SIGTERM", "SIGINT", "SIGHUP"]) process.on(s, () => stop(s));
  process.on("uncaughtException", (e) => { log("FATAL", "supervisor: " + (e?.stack || e)); });
  log("INFO", `supervisor start pid=${process.pid}`);
  spawnWorker();
}

// ── control commands ────────────────────────────────────────────────────────
function status() {
  const p = readPid();
  const alive = !!p && isWatchPidAlive(p.pid);
  const hb = readJsonOr(P.heartbeat, null);
  const age = hb ? Math.round((Date.now() - Date.parse(hb.ts)) / 1000) : null;
  const st = readJsonOr(P.state, null);
  return {
    running: alive, supervisor_pid: alive ? p.pid : null, worker_pid: hb?.pid ?? null,
    heartbeat: { path: rel(P.heartbeat), ts: hb?.ts ?? null, age_sec: age, last_cycle_at: hb?.last_cycle_at ?? null, last_error: hb?.last_error ?? null, cycles: hb?.cycles ?? null },
    delivered: st ? Object.keys(st.delivered || {}).length : 0, retrying: st ? Object.keys(st.retry || {}).length : 0,
    paths: { log: rel(P.log), state: rel(P.state), pid: rel(P.pid) },
    webhook_env_in_this_shell: { url: !!process.env.LITECODE_WP_WEBHOOK_URL, key: !!process.env.LITECODE_WP_WEBHOOK_KEY },
    stale_pidfile: !!p && !alive,
  };
}
async function start() {
  const p = readPid();
  if (p && isWatchPidAlive(p.pid)) { console.log(JSON.stringify({ ok: true, already_running: true, ...status() })); return; }
  if (p) { try { fs.unlinkSync(P.pid); } catch {} }
  const c = spawn(process.execPath, [SELF, "supervise", ...rest], { detached: true, stdio: "ignore", env: process.env, cwd: path.join(ops, "..", "..") });
  c.unref();
  for (let i = 0; i < 30; i++) { await new Promise((r) => setTimeout(r, 200)); const q = readPid(); if (q && q.pid === c.pid && readJsonOr(P.heartbeat, null)) break; }
  log("INFO", `start requested; supervisor pid=${c.pid}`);
  recordTrace(ops, wpTrace, TOOL, reg.tools_version);
  console.log(JSON.stringify({ ok: true, started: true, ...status() }));
}
async function stop() {
  const p = readPid();
  if (!p || !isWatchPidAlive(p.pid)) { if (p) try { fs.unlinkSync(P.pid); } catch {} console.log(JSON.stringify({ ok: true, was_running: false })); return; }
  process.kill(p.pid, "SIGTERM");
  for (let i = 0; i < 50 && isWatchPidAlive(p.pid); i++) await new Promise((r) => setTimeout(r, 200));
  const alive = isWatchPidAlive(p.pid);
  log("INFO", `stop requested; supervisor pid=${p.pid} ${alive ? "still alive" : "stopped"}`);
  recordTrace(ops, wpTrace, TOOL, reg.tools_version);
  console.log(JSON.stringify({ ok: !alive, was_running: true, supervisor_pid: p.pid, stopped: !alive }));
  if (alive) process.exit(1);
}

switch (cmd) {
  case "run": await runWorker(true); break;
  case "once": await runWorker(false); break;
  case "supervise": runSupervisor(); break;
  case "start": await start(); break;
  case "stop": await stop(); break;
  case "status": console.log(JSON.stringify({ ok: true, ...status() }, null, 2)); break;
  default:
    console.error("usage: wp_watch.mjs start|stop|status|supervise|run|once [--interval-sec N] [--heartbeat-sec N] [--force-timeout-scan] [--wp WP-xxx]");
    process.exit(2);
}

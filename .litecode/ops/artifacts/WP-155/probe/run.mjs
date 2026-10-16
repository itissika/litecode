#!/usr/bin/env node
// WP-155 open-path probe (read-only; lives in .tmp, deleted after the package).
// No dependencies: minimal RFC 6455 client over net. Monotonic clock (process.hrtime.bigint).
//
// usage: node run.mjs --bin <litecode> --corpus <dir> --scenario cold_noindex|cold_index|hot_reconnect|hot_reload
//                     --profile new|restore [--true-cold 1] [--evict-dry-run 1] --runs 3 --out <file.jsonl> [--index-seed <dir>] [--wait-warm-ms 600000] [--seed-home <dir>]
//
// Per run (one JSONL record), all times in ms relative to T0 (= WS upgrade request sent):
//   spawn (negative), ready (LITECODE_READY line), ws_open, frames[] {method|id, t}, req[] {method, sent, resp},
//   t_int (last proxy response: session/list + session/subscribe of the newest session),
//   proc[] {pid, comm, args, first_seen} (children appearing under serve: code-search-worker, language servers),
//   cs_warm (first /api/workspace/engines poll showing code_search Warm), peak_rss_kb (serve+children), log_path.
import { spawn } from "node:child_process";
import net from "node:net";
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import http from "node:http";
import { execFileSync } from "node:child_process";

const args = Object.fromEntries(process.argv.slice(2).reduce((a, v, i, arr) => (v.startsWith("--") ? [...a, [v.slice(2), arr[i + 1]]] : a), []));
const BIN = path.resolve(args.bin), CORPUS = path.resolve(args.corpus), SCEN = args.scenario;
const RUNS = Number(args.runs ?? 3), OUT = path.resolve(args.out);
const TRUE_COLD = "true-cold" in args; // --true-cold 1 : evict page cache before spawn (cold_noindex / cold_index)
const MODEL_DIR = process.env.LITECODE_MODEL_DIR ?? "/workspace/litecode/models/ibm-granite/granite-embedding-97m-multilingual-r2";
const WEB_DIST = process.env.LITECODE_WEB_DIST ?? "/workspace/litecode/.tmp/wp154/webstub";
const MINCORE = "/workspace/litecode/.litecode/ops/artifacts/WP-155/mincore.py"; // Session's evict.md tool (fsync + POSIX_FADV_DONTNEED, mincore check)
function sharedLibs(bin) { // ldd of the binary under test (ORT is linked in; system libs are shared by every process)
  try { return execFileSync("ldd", [bin], { encoding: "utf8" }).split("\n").map((l) => /=>\s+(\/\S+)|^\s*(\/\S+)/.exec(l)).filter(Boolean).map((m) => m[1] ?? m[2]); } catch { return []; }
}
// Returns {targets, before, after, verified}: mincore readings before and after eviction (after = the reading the run starts with).
function evictPageCache(extra = []) {
  const targets = [BIN, ...sharedLibs(BIN), MODEL_DIR, CORPUS, WEB_DIST, ...extra].filter((p) => fs.existsSync(p));
  const run = (cmd) => { try { return { code: 0, out: execFileSync("python3", [MINCORE, ...cmd, ...targets], { encoding: "utf8" }) }; } catch (e) { return { code: e.status, out: String(e.stdout ?? "") }; } };
  const parse = (o) => { try { return JSON.parse(o.trim().split("\n").at(-1)); } catch { return { raw: o.slice(0, 2000) }; } };
  const before = parse(run(["check"]).out);
  const ev = run(["evict", "--verify"]);
  const after = parse(ev.out); const libs = new Set(sharedLibs(BIN));
  const ownStill = Object.keys(after.still_resident ?? {}).filter((p) => !libs.has(p));
  // System libs are mapped by every running process, so they stay resident; reported as-is (evict.md limitation).
  return { targets, before, after, verified: ev.code === 0, verified_own_files: ownStill.length === 0, own_still_resident: ownStill };
}
const PROFILE = args.profile ?? "restore"; // new | restore
const WAIT_WARM = Number(args["wait-warm-ms"] ?? 600000);
const RSS_LIMIT_KB = 3670016; // 3.5 GiB watchdog over serve + children
const now = () => Number(process.hrtime.bigint() / 1000n) / 1000; // ms, monotonic
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function memAvailKb() { return Number(/MemAvailable:\s+(\d+)/.exec(fs.readFileSync("/proc/meminfo", "utf8"))[1]); }
function busy() {
  for (const p of fs.readdirSync("/proc").filter((d) => /^\d+$/.test(d))) {
    try { const c = fs.readFileSync(`/proc/${p}/comm`, "utf8").trim(); if (c === "cargo" || c === "rustc") return c; } catch {}
  }
  return null;
}
function children(pid) { // all descendants
  const all = [];
  for (const p of fs.readdirSync("/proc").filter((d) => /^\d+$/.test(d))) {
    try { const st = fs.readFileSync(`/proc/${p}/stat`, "utf8"); const ppid = Number(st.slice(st.lastIndexOf(")") + 2).split(" ")[1]); all.push([Number(p), ppid]); } catch {}
  }
  const out = []; const q = [pid];
  while (q.length) { const x = q.shift(); for (const [p, pp] of all) if (pp === x) { out.push(p); q.push(p); } }
  return out;
}
const rssKb = (p) => { try { return Number(/VmRSS:\s+(\d+)/.exec(fs.readFileSync(`/proc/${p}/status`, "utf8"))?.[1] ?? 0); } catch { return 0; } };
const cmdline = (p) => { try { return fs.readFileSync(`/proc/${p}/cmdline`, "utf8").split("\0").join(" ").trim(); } catch { return ""; } };

function wsConnect(port, onMsg) { // returns {sock, send, close, opened: Promise<t>}
  const sock = net.connect(port, "127.0.0.1");
  const key = crypto.randomBytes(16).toString("base64");
  let buf = Buffer.alloc(0), upgraded = false, resolveOpen, rejectOpen;
  const opened = new Promise((r, j) => { resolveOpen = r; rejectOpen = j; });
  const tSent = { t: 0 };
  sock.on("connect", () => {
    tSent.t = now();
    sock.write(`GET /ws HTTP/1.1\r\nHost: 127.0.0.1:${port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: ${key}\r\nSec-WebSocket-Version: 13\r\nOrigin: http://127.0.0.1:${port}\r\n\r\n`);
  });
  sock.on("data", (d) => {
    buf = Buffer.concat([buf, d]);
    if (!upgraded) {
      const i = buf.indexOf("\r\n\r\n"); if (i < 0) return;
      const head = buf.slice(0, i).toString(); buf = buf.slice(i + 4);
      if (!head.startsWith("HTTP/1.1 101")) return rejectOpen(new Error(head.split("\r\n")[0]));
      upgraded = true; resolveOpen(now());
    }
    for (;;) { // server frames are unmasked
      if (buf.length < 2) return;
      let len = buf[1] & 0x7f, off = 2;
      if (len === 126) { if (buf.length < 4) return; len = buf.readUInt16BE(2); off = 4; }
      else if (len === 127) { if (buf.length < 10) return; len = Number(buf.readBigUInt64BE(2)); off = 10; }
      if (buf.length < off + len) return;
      const op = buf[0] & 0x0f, payload = buf.slice(off, off + len); buf = buf.slice(off + len);
      if (op === 1) onMsg(now(), payload.toString());
    }
  });
  const send = (obj) => {
    const p = Buffer.from(JSON.stringify(obj)); const mask = crypto.randomBytes(4);
    const head = p.length < 126 ? Buffer.from([0x81, 0x80 | p.length]) : Buffer.concat([Buffer.from([0x81, 0x80 | 126]), Buffer.from([p.length >> 8, p.length & 255])]);
    const m = Buffer.from(p.map((b, i) => b ^ mask[i % 4]));
    sock.write(Buffer.concat([head, mask, m]));
  };
  return { sock, send, close: () => sock.destroy(), opened, tSent };
}

function httpGet(port, p) {
  return new Promise((res) => {
    const t0 = now();
    http.get({ host: "127.0.0.1", port, path: p, headers: { Origin: `http://127.0.0.1:${port}` } }, (r) => {
      let b = ""; r.on("data", (c) => (b += c)); r.on("end", () => res({ t0, t1: now(), status: r.statusCode, body: b }));
    }).on("error", (e) => res({ t0, t1: now(), status: 0, body: String(e) }));
  });
}

// One client session: T0 = upgrade sent; proxy = session/list then session/subscribe(newest).
async function clientOpen(port, profile = PROFILE) {
  const rec = { frames: [], req: [] }; const pending = new Map(); let nextId = 1;
  let resolveHello; const hello = new Promise((r) => (resolveHello = r));
  const c = wsConnect(port, (t, s) => {
    let m; try { m = JSON.parse(s); } catch { return; }
    rec.frames.push({ t, method: m.method ?? null, id: m.id ?? null });
    if (m.method === "server/hello") resolveHello(t);
    if (m.id != null && pending.has(m.id)) { pending.get(m.id)(t, m); pending.delete(m.id); }
  });
  const call = (method, params) => new Promise((r) => {
    const id = nextId++; const sent = now(); pending.set(id, (t, m) => { rec.req.push({ method, sent, resp: t, error: m.error?.message ?? null }); r(m); });
    c.send({ jsonrpc: "2.0", id, method, params });
  });
  rec.ws_open = await c.opened; rec.T0 = c.tSent.t;
  rec.hello = await Promise.race([hello, sleep(60000).then(() => null)]);
  // Input-box unlock proxy (front end, 2026-10-14): new profile = reply to session/new after hello;
  // restore layout = reply to session/list (subscribe of the newest session is reported too).
  // Neither waits for SessionSnapshot.
  rec.profile = profile;
  if (profile === "new") {
    const r = await call("session/new", {});
    rec.new_session = r.result?.session_id ?? null; rec.t_new = rec.req.at(-1).resp;
    rec.t_int = rec.t_new; rec.t_int_kind = "session/new reply";
  } else {
    const list = await call("session/list", {});
    rec.t_list = rec.req.at(-1).resp;
    const sessions = list.result?.sessions ?? [];
    const newest = sessions.map((s) => s.session_id ?? s.id).filter(Boolean)[0];
    if (newest) { await call("session/subscribe", { session_id: newest }); rec.t_subscribe = rec.req.at(-1).resp; }
    rec.t_int = rec.t_list; rec.t_int_kind = "session/list reply";
    rec.newest_session = newest ?? null; rec.session_count = sessions.length;
  }
  return { rec, c };
}

function rel(rec, T0) { // make every timestamp relative to T0
  const r = (x) => (x == null ? null : Math.round((x - T0) * 100) / 100);
  return { ...rec, ws_open: r(rec.ws_open), hello: r(rec.hello), t_int: r(rec.t_int), t_new: r(rec.t_new), t_list: r(rec.t_list), t_subscribe: r(rec.t_subscribe), T0: undefined,
    frames: rec.frames.map((f) => ({ ...f, t: r(f.t) })), req: rec.req.map((q) => ({ ...q, sent: r(q.sent), resp: r(q.resp) })) };
}

async function runOnce(runIdx) {
  if (busy()) throw new Error("cargo/rustc running; refusing");
  if (memAvailKb() < 6291456) throw new Error("MemAvailable < 6GiB; refusing");
  const home = path.join(path.dirname(OUT), `home-${SCEN}-${runIdx}`);
  fs.rmSync(home, { recursive: true, force: true }); fs.mkdirSync(home, { recursive: true });
  if (args["seed-home"]) fs.cpSync(args["seed-home"], home, { recursive: true });
  const lc = path.join(CORPUS, ".litecode");
  if (SCEN === "cold_noindex") { fs.rmSync(lc, { recursive: true, force: true }); }
  if (SCEN === "cold_index") { fs.rmSync(lc, { recursive: true, force: true }); fs.mkdirSync(lc, { recursive: true }); fs.cpSync(path.join(path.resolve(args["index-seed"]), "index"), path.join(lc, "index"), { recursive: true }); }
  fs.mkdirSync(lc, { recursive: true });
  fs.writeFileSync(path.join(lc, "engines.json"), JSON.stringify({ version: 1, lsp: { desired: true, servers: ["rust-analyzer"] }, retrieval: { desired: true } }, null, 2));

  let evict = null;
  if (TRUE_COLD) { // after every write (corpus prep, seed copy, engines.json), right before spawn; nothing reads these files in between
    if (!SCEN.startsWith("cold_")) throw new Error("--true-cold applies to cold_noindex / cold_index only");
    evict = evictPageCache([home]);
  }
  const tSpawn = now();
  const child = spawn("taskset", ["-c", "0-3", BIN, "serve", "--bind", "127.0.0.1:0", "--loopback-only", "--shutdown-on-stdin-eof", "--workspace", CORPUS],
    { env: { ...process.env, HOME: home, LITECODE_HOME: home, LITECODE_WEB_DIST: process.env.LITECODE_WEB_DIST ?? "/workspace/litecode/.tmp/wp154/webstub", LITECODE_MODEL_DIR: process.env.LITECODE_MODEL_DIR ?? "/workspace/litecode/models/ibm-granite/granite-embedding-97m-multilingual-r2" }, stdio: ["pipe", "pipe", "pipe"] });
  let tReady = null, port = null, peak = 0, killed = null; const procs = new Map();
  child.stdout.on("data", (d) => { const m = /LITECODE_READY http:\/\/127\.0\.0\.1:(\d+)\//.exec(d.toString()); if (m && !port) { port = Number(m[1]); tReady = now(); } });
  const stderrLog = fs.createWriteStream(path.join(path.dirname(OUT), `serve-${SCEN}-${runIdx}.stderr`)); child.stderr.pipe(stderrLog);
  const watch = setInterval(() => { // 10 ms /proc poll: child appearance + RSS watchdog
    const kids = children(child.pid); let total = rssKb(child.pid);
    for (const k of kids) { total += rssKb(k); if (!procs.has(k)) procs.set(k, { pid: k, args: cmdline(k).slice(0, 200), first_seen: now() }); }
    peak = Math.max(peak, total);
    if (total > RSS_LIMIT_KB && !killed) { killed = `WATCHDOG rss=${total}kB`; for (const k of [child.pid, ...kids]) { try { process.kill(k, "SIGKILL"); } catch {} } }
  }, 10);
  const deadline = now() + 120000;
  while (!port && now() < deadline && !killed) await sleep(2);
  if (!port) { clearInterval(watch); child.kill("SIGKILL"); throw new Error(killed ?? "no LITECODE_READY in 120 s"); }

  const { rec, c } = await clientOpen(port);
  const T0 = rec.T0; const out = { run: runIdx, scenario: SCEN + (TRUE_COLD ? "+true_cold" : ""), evict, profile: PROFILE, corpus: CORPUS, spawn: tSpawn - T0, ready: tReady - T0, ...rel(rec, T0) };

  // code_search Warm (engine state from the existing HTTP endpoint), polled every 50 ms
  let csWarm = null; const warmDeadline = now() + WAIT_WARM;
  while (now() < warmDeadline && !killed) {
    const r = await httpGet(port, "/api/workspace/engines"); out.engines_last = r.body.slice(0, 400);
    if (/"code_search"[^}]*"(state|status)"\s*:\s*"(Warm|warm)"/.test(r.body)) { csWarm = r.t1 - T0; break; }
    await sleep(50);
  }
  out.cs_warm = csWarm;

  // --wait-index <seed-dir>: one-time first build (R4): wait until the refresh job and queue are gone,
  // record t_index_done, then copy .litecode/index to the seed used by every cold_index run.
  if (args["wait-index"]) {
    const idx = path.join(lc, "index"); const idxDeadline = now() + Number(args["wait-index-ms"] ?? 14400000);
    let seen = false;
    while (now() < idxDeadline && !killed) {
      const job = fs.existsSync(path.join(idx, "job.json")), queue = fs.existsSync(path.join(idx, "queue.json"));
      if (job || queue) seen = true;
      if (seen && !job && !queue && fs.existsSync(path.join(idx, "checkpoint.json"))) { out.t_index_done = now() - T0; break; }
      await sleep(1000);
    }
    try { out.index_meta = JSON.parse(fs.readFileSync(path.join(idx, "meta.json"), "utf8")); } catch {}
    if (out.t_index_done != null) { fs.rmSync(args["wait-index"], { recursive: true, force: true }); fs.cpSync(idx, path.join(args["wait-index"], "index"), { recursive: true }); }
  }

  if (SCEN === "hot_reconnect" || SCEN === "hot_reload") { // same process: close and reopen
    c.close(); await sleep(500);
    let reload = null;
    if (SCEN === "hot_reload") { // page reload proxy: index.html + the new WS (asset/JS time not covered)
      const tReloadStart = now(); const h = await httpGet(port, "/"); reload = { start: tReloadStart, index_html: h.t1, status: h.status };
    }
    const second = await clientOpen(port, args["hot-profile"] ?? PROFILE); const T0h = reload ? reload.start : second.rec.T0;
    out.hot = { ...rel(second.rec, T0h), reload: reload && { index_html: reload.index_html - T0h, status: reload.status } };
    second.c.close();
  } else c.close();

  clearInterval(watch);
  out.proc = [...procs.values()].map((p) => ({ ...p, first_seen: Math.round((p.first_seen - T0) * 100) / 100 }));
  out.peak_rss_kb = peak; out.watchdog = killed;
  out.log_path = path.join(lc, "logs", "litecode.log");
  out.log_copy = path.join(path.dirname(OUT), `litecode-${SCEN}-${runIdx}.log`);
  child.stdin.end(); // --shutdown-on-stdin-eof
  const t = now(); while (now() - t < 30000) { try { process.kill(child.pid, 0); await sleep(50); } catch { break; } }
  for (const k of [child.pid, ...children(child.pid)]) { try { process.kill(k, "SIGKILL"); } catch {} }
  try { fs.copyFileSync(out.log_path, out.log_copy); } catch {}
  out.leftover = children(1).filter((p) => /code-search-worker|rust-analyzer/.test(cmdline(p)) && cmdline(p).includes(CORPUS));
  return out;
}

fs.mkdirSync(path.dirname(OUT), { recursive: true });
if (args["evict-dry-run"]) { // no serve: evict + mincore only, to validate the true_cold step
  const r = evictPageCache(); console.log(JSON.stringify({ targets: r.targets, before: { files: r.before.files, resident_pages: r.before.resident_pages, total_pages: r.before.total_pages }, after: { files: r.after.files, resident_pages: r.after.resident_pages, still_resident: r.after.still_resident }, verified: r.verified, verified_own_files: r.verified_own_files, own_still_resident: r.own_still_resident }, null, 1));
  process.exit(0);
}
for (let i = 1; i <= RUNS; i++) {
  const r = await runOnce(i);
  fs.appendFileSync(OUT, JSON.stringify(r) + "\n");
  console.log(`${SCEN} run ${i}: ready=${r.ready?.toFixed(0)} hello=${r.hello} t_int=${r.t_int} cs_warm=${r.cs_warm} peak=${r.peak_rss_kb}kB${r.watchdog ? " " + r.watchdog : ""}`);
  await sleep(3000);
}

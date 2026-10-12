// ops.ops_harden.test (v1, tools_version 15, WP-032). Everything runs in temp copies of .litecode/ops (os.tmpdir,
// removed at the end unless --keep). Real webhook env is stripped; delivery tests use a fake local HTTP receiver.
// Groups: 1 chain merge · 2 re-alert 60m · 3 single emitter · 4 escalation · 5 concurrent/crash-safe writes ·
//         6 restart-safe delivery · 7 issue_new · 8 toolchain_check · 9 old-WP compat · R replay of 2026-10-12 08:00–08:25Z
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import http from "node:http";
import { spawn, spawnSync } from "node:child_process";
import { fileURLToPath, pathToFileURL } from "node:url";
import { loadRegistry, printVersionHeader } from "./_lib.mjs";

const { reg } = loadRegistry(import.meta.url);
printVersionHeader(reg, "ops.ops_harden.test");
const here = path.dirname(fileURLToPath(import.meta.url));
const realOps = path.join(here, "..");
const json = process.argv.includes("--json"), keep = process.argv.includes("--keep");
const baseEnv = { ...process.env }; delete baseEnv.LITECODE_WP_WEBHOOK_URL; delete baseEnv.LITECODE_WP_WEBHOOK_KEY;
const results = [], tmpRoots = [];
let cur = null;
async function group(name, fn) {
  cur = { name, pass: 0, fail: [] };
  try { await fn(); } catch (e) { cur.fail.push("threw: " + (e?.stack || e)); }
  results.push(cur); if (!json) console.log(`${cur.fail.length ? "FAIL" : "ok  "} ${name} (${cur.pass} passed, ${cur.fail.length} failed)${cur.fail.map((f) => "\n     - " + f).join("")}`);
}
function ok(c, msg) { if (c) cur.pass++; else cur.fail.push(msg); }
const replayOut = [];

function fresh() {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "ops-harden-")); tmpRoots.push(root);
  const ops = path.join(root, ".litecode", "ops");
  fs.mkdirSync(ops, { recursive: true });
  for (const f of ["board.json", "board.schema.json", "validate.mjs", "log.jsonl", "tool_traces.jsonl"]) fs.copyFileSync(path.join(realOps, f), path.join(ops, f));
  fs.mkdirSync(path.join(ops, "events")); fs.writeFileSync(path.join(ops, "events", "inbox.json"), JSON.stringify({ inbox: [], updated_at: null }, null, 2));
  fs.mkdirSync(path.join(ops, "demand")); fs.copyFileSync(path.join(realOps, "demand", "board.json"), path.join(ops, "demand", "board.json"));
  fs.cpSync(path.join(realOps, "issues"), path.join(ops, "issues"), { recursive: true });
  fs.cpSync(here, path.join(ops, "tools"), { recursive: true, filter: (s) => !s.includes(".kb_nav_tmp") });
  return ops;
}
const lib = async (ops) => import(pathToFileURL(path.join(ops, "tools", "_lib.mjs")).href + "?t=" + Math.random());
const runSync = (ops, tool, args, env = baseEnv) => spawnSync(process.execPath, [path.join(ops, "tools", tool), ...args], { encoding: "utf8", env, cwd: path.join(ops, "..", "..") });
const runAsync = (ops, tool, args, env = baseEnv) => new Promise((res) => {
  const c = spawn(process.execPath, [path.join(ops, "tools", tool), ...args], { env, cwd: path.join(ops, "..", "..") });
  let out = "", err = ""; c.stdout.on("data", (d) => (out += d)); c.stderr.on("data", (d) => (err += d));
  c.on("close", (code) => res({ status: code, stdout: out, stderr: err }));
});
const board = (ops) => JSON.parse(fs.readFileSync(path.join(ops, "board.json"), "utf8"));
const inbox = (ops) => JSON.parse(fs.readFileSync(path.join(ops, "events", "inbox.json"), "utf8")).inbox;
function patch(ops, fn) { const b = board(ops); fn(b); fs.writeFileSync(path.join(ops, "board.json"), JSON.stringify(b, null, 2) + "\n"); }
function ackAll(ops, by = "gm") { const p = path.join(ops, "events", "inbox.json"); const b = JSON.parse(fs.readFileSync(p, "utf8")); for (const e of b.inbox) if (!e.acked.includes(by)) e.acked.push(by); fs.writeFileSync(p, JSON.stringify(b, null, 2)); }
const iso = (t) => new Date(t).toISOString();
/** Add raw WPs (bypassing ops.wp) — chain fixtures for timeout logic. */
function addWps(ops, list) {
  patch(ops, (b) => {
    for (const w of b.work_packages) w.status_entered_at = null; // isolate from real open WPs
    for (const x of list) b.work_packages.push({ domain: "team-ops", title: x.id, result: "r", done_when: ["d"], reviewer: x.approver, depends_on: [], blocker: null,
      handoff_path: null, artifact_path: null, progress: "", goal: "g", content: null, history: [{ ts: iso(0), actor: "gm", action: "approve", note: null }], parent: null, ...x });
  });
}
// fake receiver
async function receiver(plan = () => 200) {
  const got = [];
  const srv = http.createServer((req, res) => { let b = ""; req.on("data", (d) => (b += d)); req.on("end", () => { const body = JSON.parse(b); const code = plan(body, got.length); got.push({ body, code, headers: req.headers, onDisk: srv.checkDisk ? srv.checkDisk(body.event_id) : null }); res.writeHead(code); res.end("ok"); }); });
  await new Promise((r) => srv.listen(0, "127.0.0.1", r));
  return { srv, got, url: `http://127.0.0.1:${srv.address().port}/hook`, close: () => new Promise((r) => srv.close(r)) };
}
const T0 = Date.parse("2026-10-12T08:00:00Z");

await group("1 chain merge: one alert per chain to the actor nearest the stuck point", async () => {
  const ops = fresh(); const L = await lib(ops);
  addWps(ops, [
    { id: "WP-901", owner: "gm", approver: "andy", status: "in_progress", status_entered_at: iso(T0 - 3 * 3600e3) },
    { id: "WP-902", owner: "backend_mid", approver: "gm", parent: "WP-901", status: "in_progress", status_entered_at: iso(T0 - 2 * 3600e3) },
    { id: "WP-903", owner: "runtime", approver: "backend_mid", parent: "WP-902", status: "in_progress", status_entered_at: iso(T0 - 2 * 3600e3) },
    { id: "WP-904", owner: "context", approver: "backend_mid", parent: "WP-902", status: "done", status_entered_at: null },
  ]);
  let s = L.scanAndEmitWpTimeouts(ops, { force: true, now: new Date(T0) });
  ok(s.emitted.length === 1 && s.emitted[0].wp === "WP-903" && s.emitted[0].meta.target === "runtime" && s.emitted[0].meta.actor === "runtime", "only deepest running WP-903 alerted, actor=owner runtime");
  ok(!s.stale.some((x) => x.id === "WP-901" || x.id === "WP-902"), "parents with open children are not stuck");
  // review parent + running child (crafted): parent merged into child
  patch(ops, (b) => { b.work_packages.find((w) => w.id === "WP-902").status = "review"; });
  const ops2 = fresh(); const L2 = await lib(ops2);
  addWps(ops2, [
    { id: "WP-911", owner: "backend_mid", approver: "gm", status: "review", status_entered_at: iso(T0 - 2 * 3600e3) },
    { id: "WP-912", owner: "runtime", approver: "backend_mid", parent: "WP-911", status: "awaiting_approval", status_entered_at: iso(T0 - 2 * 3600e3) },
  ]);
  s = L2.scanAndEmitWpTimeouts(ops2, { force: true, now: new Date(T0) });
  ok(s.emitted.length === 1 && s.emitted[0].wp === "WP-912" && s.emitted[0].meta.target === "backend_mid" && s.emitted[0].meta.merged.join() === "WP-911", "stale ancestor merged into deepest stale descendant (awaiting → approver)");
  // child closed → parent in_progress now stuck → owner
  const ops3 = fresh(); const L3 = await lib(ops3);
  addWps(ops3, [
    { id: "WP-921", owner: "backend_mid", approver: "gm", status: "in_progress", status_entered_at: iso(T0 - 2 * 3600e3) },
    { id: "WP-922", owner: "runtime", approver: "backend_mid", parent: "WP-921", status: "done", status_entered_at: null },
  ]);
  s = L3.scanAndEmitWpTimeouts(ops3, { force: true, now: new Date(T0) });
  ok(s.emitted.length === 1 && s.emitted[0].wp === "WP-921" && s.emitted[0].meta.target === "backend_mid", "parent with all children closed → its owner");
  ok(/^E-\d+-\d+-WP921$/.test(s.emitted[0].id) && s.emitted[0].meta.matter_key === `WP-921|in_progress|${iso(T0 - 2 * 3600e3)}|backend_mid`, "matter_key + id format");
});

await group("2 re-alert: acked matter not re-sent within 60m (next cycle / 59m), re-sent after; new status = new matter", async () => {
  const ops = fresh(); const L = await lib(ops);
  addWps(ops, [{ id: "WP-931", owner: "tools", approver: "backend_mid", status: "in_progress", status_entered_at: iso(T0 - 2 * 3600e3) }]);
  let s = L.scanAndEmitWpTimeouts(ops, { force: true, now: new Date(T0) });
  ok(s.emitted.length === 1, "first alert");
  s = L.scanAndEmitWpTimeouts(ops, { force: true, now: new Date(T0 + 4 * 60e3) });
  ok(s.emitted.length === 0, "unacked: base alert not re-sent (only the one-time escalation may follow after 5m, group 4)");
  ackAll(ops);
  s = L.scanAndEmitWpTimeouts(ops, { force: true, now: new Date(T0 + 10 * 60e3) });
  ok(s.emitted.length === 0, "acked: not re-sent next cycle");
  s = L.scanAndEmitWpTimeouts(ops, { force: true, now: new Date(T0 + 59 * 60e3) });
  ok(s.emitted.length === 0, "acked: not re-sent at 59m");
  s = L.scanAndEmitWpTimeouts(ops, { force: true, now: new Date(T0 + 61 * 60e3) });
  ok(s.emitted.length === 1, "acked: re-sent after 60m");
  patch(ops, (b) => { const w = b.work_packages.find((x) => x.id === "WP-931"); w.status = "review"; w.status_entered_at = iso(T0 - 61 * 60e3); });
  s = L.scanAndEmitWpTimeouts(ops, { force: true, now: new Date(T0 + 62 * 60e3) });
  ok(s.emitted.length === 1 && s.emitted[0].meta.status === "review" && s.emitted[0].meta.target === "backend_mid", "status change → new matter immediately (review → approver)");
});

await group("3 single emitter: tick only reports while watcher healthy; concurrent emitters → one event", async () => {
  const ops = fresh();
  addWps(ops, [{ id: "WP-941", owner: "tools", approver: "backend_mid", status: "in_progress", status_entered_at: iso(Date.now() - 2 * 3600e3) }]);
  fs.mkdirSync(path.join(ops, ".tmp"), { recursive: true });
  fs.writeFileSync(path.join(ops, ".tmp", "wp_watch.heartbeat.json"), JSON.stringify({ pid: 1, ts: new Date().toISOString() }));
  let r = runSync(ops, "tick.mjs", ["--force-timeout-scan", "--no-autostart"]);
  const out = JSON.parse(r.stdout.slice(r.stdout.indexOf("{")));
  ok(r.status === 0 && out.wp_timeout.report_only === true && out.wp_timeout.stale.length === 1 && inbox(ops).length === 0, "healthy watcher → tick reports stale but emits nothing");
  fs.rmSync(path.join(ops, ".tmp", "wp_watch.heartbeat.json"));
  const rs = await Promise.all([1, 2, 3, 4].map(() => runAsync(ops, "tick.mjs", ["--force-timeout-scan", "--no-autostart"])));
  ok(rs.every((x) => x.status === 0), "4 concurrent fallback ticks exit 0");
  const ev = inbox(ops);
  ok(ev.filter((e) => e.type === "team_ops.wp_timeout").length === 1, `exactly one timeout event from 4 concurrent emitters (got ${ev.filter((e) => e.type === "team_ops.wp_timeout").length})`);
  ok(ev.filter((e) => e.type === "team_ops.watcher_down").length === 1, "one watcher_down (dedup under lock)");
});

await group("4 escalation unchanged: unacked >5m → parent approver once; top stops at gm; never andy", async () => {
  const ops = fresh(); const L = await lib(ops);
  addWps(ops, [
    { id: "WP-951", owner: "gm", approver: "andy", status: "in_progress", status_entered_at: iso(T0 - 3 * 3600e3) },
    { id: "WP-952", owner: "backend_mid", approver: "gm", parent: "WP-951", status: "in_progress", status_entered_at: iso(T0 - 3 * 3600e3) },
    { id: "WP-953", owner: "runtime", approver: "backend_mid", parent: "WP-952", status: "review", status_entered_at: iso(T0 - 2 * 3600e3) },
  ]);
  let s = L.scanAndEmitWpTimeouts(ops, { force: true, now: new Date(T0) });
  ok(s.emitted.length === 1 && s.emitted[0].meta.target === "backend_mid", "review → approver backend_mid");
  s = L.scanAndEmitWpTimeouts(ops, { force: true, now: new Date(T0 + 4 * 60e3) });
  ok(s.emitted.length === 0, "no escalation before 5m");
  s = L.scanAndEmitWpTimeouts(ops, { force: true, now: new Date(T0 + 6 * 60e3) });
  ok(s.emitted.length === 1 && s.emitted[0].meta.escalated && s.emitted[0].meta.target === "gm", "escalated to parent approver gm");
  s = L.scanAndEmitWpTimeouts(ops, { force: true, now: new Date(T0 + 30 * 60e3) });
  ok(s.emitted.length === 0, "escalate once");
  const ops2 = fresh(); const L2 = await lib(ops2);
  addWps(ops2, [{ id: "WP-961", owner: "gm", approver: "andy", status: "awaiting_approval", status_entered_at: iso(T0 - 2 * 3600e3) }]);
  s = L2.scanAndEmitWpTimeouts(ops2, { force: true, now: new Date(T0) });
  const s2 = L2.scanAndEmitWpTimeouts(ops2, { force: true, now: new Date(T0 + 10 * 60e3) });
  ok(s.emitted[0]?.meta.target === "gm" && s2.emitted.length === 0, "top-level approver andy → gm only");
  ok(!/"target":\s*"andy"/.test(fs.readFileSync(path.join(ops, "events", "inbox.json"), "utf8") + fs.readFileSync(path.join(ops2, "events", "inbox.json"), "utf8")), "no target andy");
});

await group("5 concurrent + crash-safe writes: 20 emits + acks lose nothing; stale lock & torn tmp harmless", async () => {
  const ops = fresh();
  const rs = await Promise.all(Array.from({ length: 20 }, (_, i) => runAsync(ops, "emit_event.mjs", ["--type", "team_ops.test", "--source", "t", "--summary", "e" + i])));
  ok(rs.every((r) => r.status === 0), "20 emit_event exit 0: " + rs.filter((r) => r.status).map((r) => r.stderr.slice(0, 120)).join("|"));
  let ev = inbox(ops);
  ok(ev.length === 20 && new Set(ev.map((e) => e.summary)).size === 20 && new Set(ev.map((e) => e.id)).size === 20, `all 20 present, unique ids (got ${ev.length})`);
  const acks = await Promise.all(ev.slice(0, 10).map((e) => runAsync(ops, "ack_event.mjs", ["--id", e.id, "--by", "gm"])));
  ok(acks.every((r) => r.status === 0) && inbox(ops).filter((e) => e.acked.includes("gm")).length === 10, "10 concurrent acks all applied");
  // stale lock from a dead pid + leftover torn tmp file
  fs.writeFileSync(path.join(ops, "events", ".inbox.lock"), JSON.stringify({ pid: 999999, ts: "x" }));
  fs.writeFileSync(path.join(ops, "events", "inbox.json.12345.1.tmp"), "{ torn");
  const r = runSync(ops, "emit_event.mjs", ["--type", "team_ops.test", "--source", "t", "--summary", "after-crash"]);
  ok(r.status === 0 && inbox(ops).length === 21, "stale lock taken over, write succeeds, inbox valid");
  // kill -9 a writer loop mid-flight: inbox must stay valid JSON
  const c = spawn(process.execPath, ["-e", `import(${JSON.stringify(pathToFileURL(path.join(ops, "tools", "_lib.mjs")).href)}).then(L=>{for(let i=0;i<100000;i++)L.mutateInbox(${JSON.stringify(ops)},b=>{b.inbox.push({id:"K"+i,ts:new Date().toISOString(),type:"team_ops.test",source:"k",summary:"k",wp:null,acked:[]})})})`], { stdio: "ignore" });
  await new Promise((r2) => setTimeout(r2, 400)); c.kill("SIGKILL"); await new Promise((r2) => c.on("close", r2));
  let valid = true; try { JSON.parse(fs.readFileSync(path.join(ops, "events", "inbox.json"), "utf8")); } catch { valid = false; }
  ok(valid && inbox(ops).length > 21, "after SIGKILL mid-loop inbox.json is valid JSON and kept prior events");
  const r3 = runSync(ops, "emit_event.mjs", ["--type", "team_ops.test", "--source", "t", "--summary", "after-kill"]);
  ok(r3.status === 0, "writes continue after SIGKILL (dead-pid lock recovered)");
});

await group("6 restart-safe delivery: on disk before POST; failed → retried after restart; delivered exactly once; all alert types carry target", async () => {
  const ops = fresh();
  addWps(ops, [{ id: "WP-971", owner: "tools", approver: "backend_mid", status: "in_progress", status_entered_at: iso(Date.now() - 2 * 3600e3) }]);
  let fail = true;
  const rx = await receiver(() => (fail ? 500 : 200));
  rx.srv.checkDisk = (id) => fs.readFileSync(path.join(ops, "events", "inbox.json"), "utf8").includes(id) && fs.readFileSync(path.join(ops, "events", "outbox.jsonl"), "utf8").includes(id);
  const env = { ...baseEnv, LITECODE_WP_WEBHOOK_URL: rx.url, LITECODE_WP_WEBHOOK_KEY: "test-key-not-secret" };
  // approval request through ops.wp (top-level, approver andy → target gm)
  runSync(ops, "wp.mjs", ["new", "--as", "gm", "--title", "t", "--goal", "g", "--owner", "gm", "--approver", "andy"], env);
  const top = board(ops).work_packages.at(-1).id;
  runSync(ops, "wp.mjs", ["fill", "--as", "gm", "--id", top, "--text", "c"], env);
  runSync(ops, "wp.mjs", ["submit", "--as", "gm", "--id", top], env);
  let r = await runAsync(ops, "wp_watch.mjs", ["once", "--force-timeout-scan"], env);
  ok(r.status === 0 && rx.got.length === 2 && rx.got.every((g) => g.code === 500), "first run: 2 POSTs (timeout + approval_requested) both 500: " + r.stderr.slice(-200));
  ok(rx.got.every((g) => g.onDisk === true), "each event was in inbox.json AND outbox.jsonl before its POST arrived");
  const st = JSON.parse(fs.readFileSync(path.join(ops, ".tmp", "wp_watch.state.json"), "utf8"));
  ok(Object.keys(st.retry).length === 2 && Object.keys(st.delivered).length === 0, "failures recorded for retry (durable state)");
  // "restart": clear retry back-off (as if time passed), receiver now OK, new worker process
  st.retry = Object.fromEntries(Object.entries(st.retry).map(([k, v]) => [k, { ...v, next_at: iso(0) }])); fs.writeFileSync(path.join(ops, ".tmp", "wp_watch.state.json"), JSON.stringify(st));
  fail = false;
  r = await runAsync(ops, "wp_watch.mjs", ["once", "--force-timeout-scan"], env);
  const ok200 = rx.got.filter((g) => g.code === 200);
  ok(r.status === 0 && ok200.length === 2, "after restart both delivered");
  r = await runAsync(ops, "wp_watch.mjs", ["once", "--force-timeout-scan"], env);
  ok(rx.got.filter((g) => g.code === 200).length === 2, "third run: no duplicate delivery");
  const tmo = ok200.find((g) => g.body.type === "team_ops.wp_timeout"), apr = ok200.find((g) => g.body.type === "team_ops.approval_requested");
  ok(tmo && tmo.body.target === "tools" && tmo.body.actor === "tools" && tmo.body.matter_key && Array.isArray(tmo.body.merged), "timeout payload: target=tools (non-gm still via webhook), actor, matter_key, merged");
  ok(apr && apr.body.target === "gm" && apr.body.wp_id === top, "approval_requested delivered, target gm (approver andy → gm)");
  ok(/^sha256=[0-9a-f]{64}$/.test(tmo.headers["x-litecode-signature"]) && tmo.headers["x-litecode-event-id"] === tmo.body.event_id, "HMAC + event-id headers");
  for (const k of ["event_id", "wp_id", "status", "age_minutes", "owner", "reviewer", "status_entered_at", "summary", "ts", "level", "parent", "path", "escalated"]) ok(k in tmo.body, "payload keeps field " + k);
  await rx.close();
});

await group("7 issue_new: concurrent allocation unique; reserved skipped; never overwrites; validate catches dup/mismatch", async () => {
  const ops = fresh();
  const idir = path.join(ops, "issues");
  for (const f of fs.readdirSync(idir)) if (f.startsWith("ISSUE-")) fs.rmSync(path.join(idir, f));
  for (const n of ["001", "002"]) fs.writeFileSync(path.join(idir, `ISSUE-${n}.json`), JSON.stringify({ id: `ISSUE-${n}` }));
  fs.writeFileSync(path.join(idir, ".reserved.json"), JSON.stringify({ reserved: [{ id: "ISSUE-003", reason: "t" }] }));
  // git HEAD in this temp copy: not a repo → only files + reserved count
  const rs = await Promise.all(Array.from({ length: 10 }, (_, i) => runAsync(ops, "issue_new.mjs", ["--as", "tools", "--title", "t" + i, "--domain", "tools"])));
  const ids = rs.map((r) => { try { return JSON.parse(r.stdout.slice(r.stdout.indexOf("{"))).id; } catch { return null; } });
  ok(rs.every((r) => r.status === 0), "10 concurrent allocations exit 0");
  ok(new Set(ids).size === 10 && !ids.includes("ISSUE-003") && ids.sort()[0] === "ISSUE-004" && ids.sort()[9] === "ISSUE-013", "ids unique, 004..013, reserved 003 skipped: " + ids.join(","));
  const titles = new Set(fs.readdirSync(idir).filter((f) => f.startsWith("ISSUE-")).map((f) => JSON.parse(fs.readFileSync(path.join(idir, f), "utf8")).title).filter(Boolean));
  ok(titles.size === 10, "no file overwritten (10 distinct titles on disk)");
  fs.writeFileSync(path.join(idir, "ISSUE-050.json"), JSON.stringify({ id: "ISSUE-004" }));
  let v = runSync(ops, "validate_all.mjs", []);
  ok(v.status !== 0 && /does not match file name/.test(v.stderr) && /duplicate id ISSUE-004/.test(v.stderr), "validate_all catches mismatch + duplicate");
  fs.rmSync(path.join(idir, "ISSUE-050.json")); fs.writeFileSync(path.join(idir, "ISSUE-003.json"), JSON.stringify({ id: "ISSUE-003" }));
  v = runSync(ops, "validate_all.mjs", []);
  ok(v.status !== 0 && /reserved/.test(v.stderr), "validate_all rejects a reserved id file");
});

await group("8 toolchain_check: broken rustup proxy on PATH → FAIL; wrapper → ok", async () => {
  const ops = fresh();
  const root = path.dirname(path.dirname(ops));
  const bad = path.join(root, "badbin"), good = path.join(root, "goodbin"); fs.mkdirSync(bad); fs.mkdirSync(good);
  fs.writeFileSync(path.join(bad, "cargo"), "#!/bin/sh\necho \"error: rustup could not choose a version of cargo to run, because one wasn't specified explicitly, and no default is configured.\" >&2; exit 1\n", { mode: 0o755 });
  fs.writeFileSync(path.join(bad, "rustup"), "#!/bin/sh\necho 'error: no active toolchain' >&2; exit 1\n", { mode: 0o755 });
  fs.writeFileSync(path.join(good, "cargo"), "#!/bin/sh\necho 'cargo 1.99.0 (fake)'\n", { mode: 0o755 });
  fs.writeFileSync(path.join(good, "rustup"), "#!/bin/sh\necho 'stable-x86_64-unknown-linux-gnu (default)'\n", { mode: 0o755 });
  const envBad = { ...baseEnv, PATH: `${bad}:/usr/local/bin:/usr/bin:/bin`, HOME: path.join(root, "home") }; delete envBad.RUSTUP_HOME; delete envBad.CARGO_HOME;
  let r = runSync(ops, "toolchain_check.mjs", ["--json"], envBad);
  let j = JSON.parse(r.stdout.slice(r.stdout.indexOf("{")));
  ok(r.status === 1 && j.results.find((x) => x.flavour === "inherited").ok === false && j.results.find((x) => x.flavour.startsWith("bash -c")).ok === false, "broken proxy first on PATH → inherited + bash -c FAIL, exit 1");
  const envGood = { ...envBad, PATH: `${good}:/usr/local/bin:/usr/bin:/bin` };
  r = runSync(ops, "toolchain_check.mjs", ["--json"], envGood);
  j = JSON.parse(r.stdout.slice(r.stdout.indexOf("{")));
  ok(j.results.find((x) => x.flavour === "inherited").ok && j.results.find((x) => x.flavour.startsWith("bash -c")).ok, "working cargo first on PATH → inherited + bash -c ok");
});

await group("9 old-WP compat: 19 old WPs validate/gate unchanged; old alert summary; old WPs re-alert after 60m, never escalate", async () => {
  const ops = fresh(); const L = await lib(ops);
  ok(spawnSync(process.execPath, [path.join(ops, "validate.mjs")]).status === 0, "validate OK");
  const olds = board(ops).work_packages.filter((w) => !w.goal && !w.parent && !Array.isArray(w.history));
  ok(olds.length === 19, "19 old WPs");
  for (const id of ["WP-001", "WP-003", "WP-017", "WP-019"]) { const r = runSync(ops, "wp_gate.mjs", ["--id", id, "--to", "review", "--dry-run", "--source", "gm"]); ok(r.status === 0 && /"dry_run":true/.test(r.stdout), id + " gate dry-run ok"); }
  patch(ops, (b) => { for (const w of b.work_packages) w.status_entered_at = null; const o = b.work_packages.find((w) => w.id === "WP-017"); o.status = "review"; o.status_entered_at = iso(T0 - 2 * 3600e3); });
  let s = L.scanAndEmitWpTimeouts(ops, { force: true, now: new Date(T0) });
  ok(s.emitted.length === 1 && /^WP-017 review open \d+m since .* owner=demand_officer reviewer=gm$/.test(s.emitted[0].summary) && s.emitted[0].meta.target === "gm", "old summary format unchanged, target reviewer");
  s = L.scanAndEmitWpTimeouts(ops, { force: true, now: new Date(T0 + 30 * 60e3) });
  ok(s.emitted.length === 0, "old WP unacked: no escalation, no resend");
  ackAll(ops);
  s = L.scanAndEmitWpTimeouts(ops, { force: true, now: new Date(T0 + 45 * 60e3) });
  ok(s.emitted.length === 0, "old WP acked: not re-sent within 60m");
  s = L.scanAndEmitWpTimeouts(ops, { force: true, now: new Date(T0 + 61 * 60e3) });
  ok(s.emitted.length === 1 && !s.emitted[0].meta.escalated, "old WP acked: re-sent after 60m, not escalated");
  const tk = runSync(ops, "tick.mjs", ["--no-watcher-check"]);
  ok(tk.status === 0 && /"awaiting_approval_wp"/.test(tk.stdout) && /"ready_wp"/.test(tk.stdout), "tick keeps old keys");
});

await group("R replay 2026-10-12 08:00–08:25Z (GM acks everything right after each cycle, as happened)", async () => {
  const scenario = [
    { id: "WP-021", owner: "gm", approver: "andy", status: "in_progress", status_entered_at: "2026-10-12T07:01:52.088Z" },
    { id: "WP-022", owner: "backend_mid", approver: "gm", parent: "WP-021", status: "in_progress", status_entered_at: "2026-10-12T07:03:36.371Z" },
    { id: "WP-023", owner: "frontend_mid", approver: "gm", parent: "WP-021", status: "done", status_entered_at: null },
    { id: "WP-024", owner: "session", approver: "backend_mid", parent: "WP-022", status: "in_progress", status_entered_at: "2026-10-12T07:05:29.817Z" },
    { id: "WP-027", owner: "tools", approver: "backend_mid", parent: "WP-022", status: "in_progress", status_entered_at: "2026-10-12T07:06:10.945Z" },
    { id: "WP-025", owner: "runtime", approver: "backend_mid", parent: "WP-022", status: "done", status_entered_at: null },
  ];
  // board transitions that really happened: WP-024 review 08:08:38 → done 08:09:01; WP-027 review 08:10:44 → done 08:16:27
  const cycles = ["08:02:24.825", "08:07:24.838", "08:14:45.189", "08:19:45.221", "08:24:45.274"].map((t) => Date.parse(`2026-10-12T${t}Z`));
  const transitions = [
    { at: Date.parse("2026-10-12T08:08:38Z"), id: "WP-024", status: "review" }, { at: Date.parse("2026-10-12T08:09:01Z"), id: "WP-024", status: "done" },
    { at: Date.parse("2026-10-12T08:10:44Z"), id: "WP-027", status: "review" }, { at: Date.parse("2026-10-12T08:16:27Z"), id: "WP-027", status: "done" },
    { at: Date.parse("2026-10-12T08:17:56Z"), id: "WP-022", status: "review" },
  ];
  async function play(libPath) {
    const ops = fresh();
    const L = await import(pathToFileURL(libPath(ops)).href + "?r=" + Math.random());
    // remove real WP-021.. so the scenario ids are ours
    patch(ops, (b) => { b.work_packages = b.work_packages.filter((w) => Number(w.id.slice(3)) < 21); });
    addWps(ops, scenario.map((x) => ({ ...x })));
    const sent = [];
    for (const c of cycles) {
      for (const t of transitions) if (t.at <= c) patch(ops, (b) => { const w = b.work_packages.find((x) => x.id === t.id); if (w.status !== t.status) { w.status = t.status; w.status_entered_at = ["review", "in_progress", "awaiting_approval"].includes(t.status) ? iso(t.at) : null; } });
      const s = L.scanAndEmitWpTimeouts(ops, { force: true, now: new Date(c), source: "replay" });
      for (const e of s.emitted) sent.push({ at: iso(c), wp: e.wp, target: e.meta?.target ?? e.meta?.reviewer ?? null });
      ackAll(ops); // GM acked each delivery within the cycle
    }
    return sent;
  }
  const oldSent = await play((ops) => path.join(ops, "tools", "fixtures", "_lib.v14.mjs"));
  const newSent = await play((ops) => path.join(ops, "tools", "_lib.mjs"));
  const win = (xs) => xs.filter((x) => x.at >= "2026-10-12T08:07" && x.at < "2026-10-12T08:22");
  replayOut.push({ v14_all: oldSent, v15_all: newSent, window_0807_0822: { v14_webhook_posts: win(oldSent).length, v14_target_gm: win(oldSent).filter((x) => x.target === "gm").length, v15_webhook_posts: win(newSent).length, v15_target_gm: win(newSent).filter((x) => x.target === "gm").length } });
  ok(win(oldSent).length === 7, `v14 replay reproduces 7 GM wakes in 08:07–08:22 (got ${win(oldSent).length})`);
  ok(win(newSent).length <= 2 && win(newSent).filter((x) => x.target === "gm").length === 0, `v15: ≤2 posts, none targeting gm (got ${JSON.stringify(win(newSent))})`);
  ok(!newSent.some((x) => x.wp === "WP-021" || x.wp === "WP-022"), "v15 never alerts parents WP-021/WP-022 while children run");
});

if (!keep) for (const r of tmpRoots) fs.rmSync(r, { recursive: true, force: true });
const failed = results.filter((g) => g.fail.length);
const passed = results.reduce((a, g) => a + g.pass, 0), failedN = results.reduce((a, g) => a + g.fail.length, 0);
if (!json && replayOut.length) console.log("REPLAY " + JSON.stringify(replayOut[0]));
console.log(JSON.stringify({ ok: failed.length === 0, groups: results.length, passed_groups: results.length - failed.length, checks_total: passed + failedN, checks_passed: passed, checks_failed: failedN, failures: failed.map((g) => ({ group: g.name, fail: g.fail })), tools_version: reg.tools_version }));
process.exit(failed.length ? 1 : 0);

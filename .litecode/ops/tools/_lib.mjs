import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync, spawn } from "node:child_process";
export function opsRoot(fromMetaUrl) {
  return path.join(path.dirname(fileURLToPath(fromMetaUrl)), "..");
}
export function loadRegistry(fromMetaUrl) {
  const regPath = path.join(path.dirname(fileURLToPath(fromMetaUrl)), "registry.json");
  return { regPath, reg: JSON.parse(fs.readFileSync(regPath, "utf8")) };
}
export function printVersionHeader(reg, toolId) {
  const t = (reg.tools || []).find((x) => x.id === toolId);
  const line = `LITECODE_OPS tools_version=${reg.tools_version} tool=${toolId} tool_version=${t ? t.version : "?"} updated_at=${reg.updated_at}`;
  console.log(line);
  return line;
}
export function recordTrace(ops, wp, toolId, toolsVersion) {
  if (!wp) return;
  const p = path.join(ops, "tool_traces.jsonl");
  fs.appendFileSync(p, JSON.stringify({ ts: new Date().toISOString(), wp, tool: toolId, tools_version: toolsVersion }) + "\n");
}
// v8: regenerate the「WP 状态」table in tools/board.md from board.json
export function renderWpStatusTable(ops) {
  const board = JSON.parse(fs.readFileSync(path.join(ops, "board.json"), "utf8"));
  const mdPath = path.join(ops, "tools", "board.md");
  const md = fs.readFileSync(mdPath, "utf8");
  const esc = (s) => String(s ?? "").replace(/\|/g, "\\|").replace(/\n/g, " ");
  // v13: rows ordered as a parent tree (children indented under their parent); old WPs are roots, row text unchanged
  const rows = wpTreeOrder(board).map(({ w, depth }) => `| ${esc(w.id)} | ${depth ? "　".repeat(depth - 1) + "└ " : ""}${esc(w.title)} | ${esc(w.status)} | ${esc(w.owner)} | ${esc(w.reviewer)} |`);
  const BEGIN = "<!-- WP_STATUS:BEGIN (auto, wp_gate/tick 生成，勿手改) -->";
  const END = "<!-- WP_STATUS:END -->";
  const block = [BEGIN, "## WP 状态", "", `> 自动生成于 ${new Date().toISOString()}，来源 \`.litecode/ops/board.json\``, "", "| id | 标题 | status | owner | reviewer |", "|---|---|---|---|---|", ...rows, END].join("\n");
  let out;
  const re = /<!-- WP_STATUS:BEGIN[^\n]*-->[\s\S]*?<!-- WP_STATUS:END -->/;
  if (re.test(md)) out = md.replace(re, block);
  else if (md.includes("\n## 最近更新")) out = md.replace("\n## 最近更新", "\n" + block + "\n\n## 最近更新");
  else out = md.trimEnd() + "\n\n" + block + "\n";
  if (out !== md) fs.writeFileSync(mdPath, out);
  return { rows: rows.length, changed: out !== md };
}


/** HKT weekday 09:45–17:45 (inclusive start, exclusive end at 17:45). */
export function isWorkHoursHkt(now = new Date()) {
  const fmt = new Intl.DateTimeFormat("en-US", {
    timeZone: "Asia/Hong_Kong",
    weekday: "short",
    hour: "2-digit",
    minute: "2-digit",
    hourCycle: "h23",
  });
  const parts = Object.fromEntries(fmt.formatToParts(now).filter((p) => p.type !== "literal").map((p) => [p.type, p.value]));
  const wd = parts.weekday;
  if (wd === "Sat" || wd === "Sun") return false;
  const mins = Number(parts.hour) * 60 + Number(parts.minute);
  return mins >= 9 * 60 + 45 && mins < 17 * 60 + 45;
}

/** Open clock statuses that can go stale. */
export const OPEN_CLOCK_STATUSES = new Set(["in_progress", "review", "awaiting_approval"]); // v13: + awaiting_approval

export function applyStatusEnteredAt(wp, to, nowIso = new Date().toISOString()) {
  if (OPEN_CLOCK_STATUSES.has(to)) {
    wp.status_entered_at = nowIso;
  } else {
    wp.status_entered_at = null;
  }
  return wp.status_entered_at;
}

// ── v13: nested WP model (WP-020) ───────────────────────────────────────────
// A WP is "new model" when it declares parent or goal (or carries history). Old WPs keep v12 behaviour.
export const WP_STATUSES = ["drafting", "awaiting_approval", "backlog", "ready", "in_progress", "review", "done", "blocked", "cancelled"];
export const CLOSED_STATUSES = new Set(["done", "cancelled", "blocked"]);
export const ESCALATE_AFTER_MS = 5 * 60 * 1000; // one wp_watch cycle
export function isNewModelWp(w) {
  return !!w && ((w.parent !== undefined && w.parent !== null) || (typeof w.goal === "string" && w.goal.length > 0) || Array.isArray(w.history));
}
/** Effective approver: approver, else reviewer (old WPs need no migration). */
export function approverOf(w) { return w ? (w.approver ?? w.reviewer ?? null) : null; }
export function wpById(board, id) { return board.work_packages.find((w) => w.id === id) || null; }
export function childrenOf(board, id) { return board.work_packages.filter((w) => w.parent === id); }
/** Root→self id chain via parent (cycle-safe). */
export function wpChain(board, id) {
  const chain = []; const seen = new Set(); let cur = wpById(board, id);
  while (cur && !seen.has(cur.id)) { seen.add(cur.id); chain.unshift(cur.id); cur = cur.parent ? wpById(board, cur.parent) : null; }
  return chain;
}
/** Board order DFS from roots (WPs without a resolvable parent). */
export function wpTreeOrder(board, rootId = null) {
  const out = []; const seen = new Set();
  const walk = (w, depth) => { if (seen.has(w.id)) return; seen.add(w.id); out.push({ w, depth }); for (const c of childrenOf(board, w.id)) walk(c, depth + 1); };
  if (rootId) { const r = wpById(board, rootId); if (r) walk(r, 0); return out; }
  for (const w of board.work_packages) if (!w.parent || !wpById(board, w.parent)) walk(w, 0);
  for (const w of board.work_packages) if (!seen.has(w.id)) walk(w, 0); // cycles (validate rejects them) still shown
  return out;
}
/** Is the WP PLAN approved in its current round (an approve after the last submit-cycle reset)? */
export function isApproved(w) {
  const h = Array.isArray(w.history) ? w.history : [];
  for (let i = h.length - 1; i >= 0; i--) {
    if (h[i].action === "approve") return true;
    if (h[i].action === "reject" || h[i].action === "reopen" || h[i].action === "submit") return false; // "return"/"deliver"/"start" keep plan approval
  }
  return false;
}
/** Escalation targets never include andy: top of the chain is gm (GM decides whether to tell Andy). */
export function escalationTarget(id) { return !id || id === "andy" ? "gm" : id; }
export function escalationPlan(board, w) {
  const target = escalationTarget(approverOf(w));
  const parent = w.parent ? wpById(board, w.parent) : null;
  const up = parent ? escalationTarget(approverOf(parent)) : "gm";
  return { target, up: up === target ? null : up };
}
export function hierarchyMeta(board, w) {
  const chain = wpChain(board, w.id);
  return { level: chain.length - 1, parent: w.parent ?? null, path: chain };
}

// ── v15 (WP-032): durable event store — lock + fsync + atomic rename; outbox before send ─────────
export const REALERT_MS_DEFAULT = 60 * 60 * 1000; // acked matter is not re-sent within 60 min (old WPs too)
export const DELIVER_TYPES = new Set(["team_ops.wp_timeout", "team_ops.approval_requested", "team_ops.acceptance_requested", "team_ops.watcher_down", "team_ops.watcher_restarted", "team_ops.request_resolved"]);
function sleepMs(ms) { Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, ms); }
function pidAlive(pid) { try { process.kill(pid, 0); return true; } catch (e) { return e.code === "EPERM"; } }
/** Exclusive lock via O_EXCL lockfile; stale (dead pid or >30s) locks are taken over. */
export function withLock(lockPath, fn, { timeoutMs = 5000, staleMs = 30000 } = {}) {
  const t0 = Date.now(); let fd = null; let wait = 5;
  while (fd === null) {
    try { fd = fs.openSync(lockPath, "wx"); fs.writeSync(fd, JSON.stringify({ pid: process.pid, ts: new Date().toISOString() })); }
    catch (e) {
      if (e.code !== "EEXIST") throw e;
      let info = null; try { info = JSON.parse(fs.readFileSync(lockPath, "utf8")); } catch {}
      let st = null; try { st = fs.statSync(lockPath); } catch {}
      const stale = (info && Number.isInteger(info.pid) && !pidAlive(info.pid)) || (st && Date.now() - st.mtimeMs > staleMs);
      if (stale) { try { fs.unlinkSync(lockPath); } catch {} continue; }
      if (Date.now() - t0 > timeoutMs) throw new Error(`lock timeout: ${lockPath} held by ${info ? info.pid : "?"}`);
      sleepMs(wait); wait = Math.min(wait * 2, 100);
    }
  }
  try { return fn(); } finally { try { fs.closeSync(fd); } catch {} try { fs.unlinkSync(lockPath); } catch {} }
}
function fsyncDir(dir) { try { const d = fs.openSync(dir, "r"); try { fs.fsyncSync(d); } finally { fs.closeSync(d); } } catch {} }
/** tmp + fsync + rename + fsync(dir): after return the content survives a crash/restart. */
export function writeFileDurable(p, text) {
  const t = `${p}.${process.pid}.${Date.now()}.tmp`;
  const fd = fs.openSync(t, "w");
  try { fs.writeSync(fd, text); fs.fsyncSync(fd); } finally { fs.closeSync(fd); }
  fs.renameSync(t, p); fsyncDir(path.dirname(p));
}
export function appendLineDurable(p, line) {
  const fd = fs.openSync(p, "a");
  try { fs.writeSync(fd, line.endsWith("\n") ? line : line + "\n"); fs.fsyncSync(fd); } finally { fs.closeSync(fd); }
}
export function eventPaths(ops) {
  const dir = path.join(ops, "events");
  return { dir, inbox: path.join(dir, "inbox.json"), outbox: path.join(dir, "outbox.jsonl"), lock: path.join(dir, ".inbox.lock") };
}
/** Read-modify-write inbox.json under the lock; fn(box) may return a value. Events whose type is in
 * DELIVER_TYPES that fn pushed are appended to outbox.jsonl (durable) BEFORE anyone can send them. */
export function mutateInbox(ops, fn) {
  const P = eventPaths(ops);
  return withLock(P.lock, () => {
    const box = JSON.parse(fs.readFileSync(P.inbox, "utf8"));
    const before = new Set(box.inbox.map((e) => e.id));
    const ret = fn(box);
    const added = box.inbox.filter((e) => !before.has(e.id));
    if (ret === false) return { ret, added: [] };
    box.updated_at = new Date().toISOString();
    writeFileDurable(P.inbox, JSON.stringify(box, null, 2) + "\n");
    for (const e of added) if (DELIVER_TYPES.has(e.type)) appendLineDurable(P.outbox, JSON.stringify({ event_id: e.id, ts: e.ts, type: e.type, wp: e.wp ?? null, target: e.meta?.target ?? e.meta?.approver ?? null }));
    return { ret, added };
  });
}
export function readOutbox(ops) {
  const p = eventPaths(ops).outbox;
  if (!fs.existsSync(p)) return [];
  return fs.readFileSync(p, "utf8").split("\n").filter(Boolean).map((l) => { try { return JSON.parse(l); } catch { return null; } }).filter(Boolean);
}
let evSeq = 0;
export function newEventId(suffix, now = Date.now()) { return "E-" + now + "-" + String(process.pid % 10000) + String(evSeq++) + (suffix ? "-" + suffix : ""); }

/** Who must act on a stuck WP: approver for awaiting_approval/review, owner for in_progress. */
export function actorOf(w) { return w.status === "in_progress" ? (w.owner ?? null) : approverOf(w); }

/**
 * v15 timeout scan (WP-032). Candidates: in_progress/review/awaiting_approval open >1h (work hours).
 * - in_progress with open children is NOT stuck (waiting on children).
 * - chain merge: a candidate with a candidate descendant is merged into that descendant (one alert per chain,
 *   to the actor nearest the stuck point); merged ids go to meta.merged.
 * - matter = wp|status|status_entered_at|actor. Unacked matter: never re-sent. Acked matter: re-sent only after
 *   realertMs (default 60 min) since its last alert. Old WPs too.
 * - escalation (new-model WPs only): base unacked >5 min → one escalated alert to parent's approver; top → gm; never andy.
 * - emit=false: report only (tick while watcher is healthy). Events are written durably (inbox + outbox) before return.
 */
export function scanAndEmitWpTimeouts(ops, { force = false, now = new Date(), source = "ops.tick", emit = true, realertMs = REALERT_MS_DEFAULT } = {}) {
  const workHours = force || isWorkHoursHkt(now);
  const board = JSON.parse(fs.readFileSync(path.join(ops, "board.json"), "utf8"));
  const hourMs = 60 * 60 * 1000;
  const cand = new Map();
  for (const w of board.work_packages) {
    if (!OPEN_CLOCK_STATUSES.has(w.status) || !w.status_entered_at) continue;
    const entered = Date.parse(w.status_entered_at);
    if (!Number.isFinite(entered)) continue;
    const ageMs = now.getTime() - entered;
    if (ageMs < hourMs) continue;
    if (w.status === "in_progress" && childrenOf(board, w.id).some((c) => !CLOSED_STATUSES.has(c.status))) continue; // waiting on children
    cand.set(w.id, { w, ageMs });
  }
  // chain merge: drop candidates that have a candidate descendant
  const mergedInto = new Map();
  for (const id of cand.keys()) {
    const chain = wpChain(board, id).slice(0, -1); // ancestors
    for (const anc of chain) if (cand.has(anc)) { mergedInto.set(anc, id); }
  }
  const stale = [];
  for (const [id, { w, ageMs }] of cand) {
    if (mergedInto.has(id)) continue;
    const merged = [...mergedInto.entries()].filter(([, d]) => d === id).map(([a]) => a);
    stale.push({ id: w.id, status: w.status, owner: w.owner, reviewer: w.reviewer, status_entered_at: w.status_entered_at, age_minutes: Math.floor(ageMs / 60000), actor: actorOf(w), merged });
  }
  if (!workHours || stale.length === 0 || !emit) {
    return { work_hours: workHours, stale, emitted: [], quiet: true, merged: Object.fromEntries(mergedInto), report_only: !emit };
  }
  const { added } = mutateInbox(ops, (box) => {
    let pushed = 0;
    for (const s of stale) {
      const w = wpById(board, s.id);
      const nm = isNewModelWp(w);
      const target = escalationTarget(s.actor);
      const matter = `${s.id}|${s.status}|${s.status_entered_at}|${target}`;
      const hm = hierarchyMeta(board, w);
      const same = (e) => e.type === "team_ops.wp_timeout" && e.wp === s.id && Date.parse(e.ts) >= Date.parse(s.status_entered_at) &&
        (e.meta?.matter_key ? e.meta.matter_key === matter : true);
      const unacked = (e) => !e.acked || e.acked.length === 0;
      const prior = box.inbox.filter((e) => same(e) && !e.meta?.escalated);
      const base = prior.find(unacked);
      const mk = (escalated, tgt) => {
        const summary = `${s.id} ${s.status} open ${s.age_minutes}m since ${s.status_entered_at} (work-hours >1h); owner=${s.owner ?? "-"} reviewer=${s.reviewer ?? "-"}` +
          (nm ? `; target=${tgt}${escalated ? " (escalated)" : ""}` : "") + (s.merged.length ? `; merged=${s.merged.join(",")}` : "");
        box.inbox.push({
          id: newEventId(s.id.replace(/\W/g, "") + (escalated ? "-esc" : ""), now.getTime()),
          ts: now.toISOString(), type: "team_ops.wp_timeout", source, summary, wp: s.id, acked: [],
          meta: { status: s.status, status_entered_at: s.status_entered_at, age_minutes: s.age_minutes, owner: s.owner, reviewer: s.reviewer,
            ...hm, target: tgt, actor: s.actor, escalated, merged: s.merged, matter_key: matter },
        });
        pushed++;
      };
      if (!base) {
        const last = prior.map((e) => Date.parse(e.ts)).sort((a, b) => b - a)[0];
        if (last !== undefined && now.getTime() - last < realertMs) continue; // acked within re-alert window
        mk(false, target); continue;
      }
      if (!nm) continue;
      const parent = w.parent ? wpById(board, w.parent) : null;
      const up = parent ? escalationTarget(approverOf(parent)) : "gm";
      if (up === target) continue;
      if (now.getTime() - Date.parse(base.ts) < ESCALATE_AFTER_MS) continue;
      if (box.inbox.some((e) => same(e) && e.meta?.escalated)) continue;
      mk(true, up);
    }
    return pushed ? true : false;
  });
  return { work_hours: workHours, stale, emitted: added, quiet: added.length === 0, merged: Object.fromEntries(mergedInto) };
}

/** Webhook body (wp_watch). v11 fields unchanged; v13 level/parent/path/target/escalated; v15 type/actor/merged/matter_key. */
export function webhookPayloadOf(ev) {
  const m = ev.meta || {};
  return {
    event_id: ev.id, wp_id: ev.wp, status: m.status ?? null, age_minutes: m.age_minutes ?? null,
    owner: m.owner ?? null, reviewer: m.reviewer ?? null, status_entered_at: m.status_entered_at ?? null,
    summary: ev.summary, ts: ev.ts,
    level: m.level ?? 0, parent: m.parent ?? null, path: m.path ?? (ev.wp ? [ev.wp] : []), target: m.target ?? m.approver ?? m.reviewer ?? "gm", escalated: !!m.escalated,
    type: ev.type, actor: m.actor ?? null, merged: m.merged ?? [], matter_key: m.matter_key ?? null,
  };
}

/** v19 liveness driver without cron/systemd: any ops.wp write calls this. If a watcher was ever set up here (heartbeat file exists) and it is
 *  dead or its heartbeat is >10 min old, spawn detached `wp_watch.mjs ensure` (rate-limited 5 min via .tmp/wp_watch.ensure.stamp) and
 *  record team_ops.watcher_restarted(source ops.wp). Never throws; no-op in test copies without .tmp/heartbeat. */
export function softEnsureWatcher(ops, { now = new Date(), staleMs = 10 * 60000 } = {}) {
  try {
    if (process.env.LITECODE_WP_NO_AUTOENSURE === "1") return { action: "disabled" };
    const wpth = watchPaths(ops);
    const hb = readJsonOr(wpth.heartbeat, null);
    if (!hb) return { action: "none", reason: "no heartbeat file" };
    const age = now.getTime() - Date.parse(hb.ts);
    const pidf = readJsonOr(wpth.pid, null);
    const alive = !!pidf && isWatchPidAlive(pidf.pid);
    if (alive && age <= staleMs) return { action: "none", reason: "healthy" };
    const stamp = path.join(wpth.tmp, "wp_watch.ensure.stamp");
    try { if (now.getTime() - fs.statSync(stamp).mtimeMs < 5 * 60000) return { action: "none", reason: "rate-limited" }; } catch {}
    fs.writeFileSync(stamp, now.toISOString());
    const c = spawn(process.execPath, [path.join(ops, "tools", "wp_watch.mjs"), "ensure"], { detached: true, stdio: "ignore", env: process.env, cwd: path.join(ops, "..", "..") });
    c.unref();
    mutateInbox(ops, (box) => {
      box.inbox.push({ id: newEventId("watcher-restarted", now.getTime()), ts: now.toISOString(), type: "team_ops.watcher_restarted", source: "ops.wp", wp: null, acked: [],
        summary: `wp_watch ${alive ? "heartbeat " + Math.round(age / 60000) + "m old" : "not running"} (last heartbeat ${hb.ts}); ops.wp triggered ensure/restart`, meta: { target: "gm", ok: null, last_heartbeat_at: hb.ts, age_sec: Math.round(age / 1000), old_pid: hb.pid ?? null, via: "ops.wp softEnsure" } });
    });
    return { action: "ensure_spawned", age_sec: Math.round(age / 1000) };
  } catch (e) { return { action: "error", error: String(e.message || e) }; }
}

/** Write board, run validate.mjs, roll back on failure. Returns { ok, stderr }. */
export function writeBoardValidated(ops, board, boardRaw) {
  const boardPath = path.join(ops, "board.json");
  fs.writeFileSync(boardPath, JSON.stringify(board, null, 2) + (boardRaw.endsWith("\n") ? "\n" : ""));
  const v = spawnSync(process.execPath, [path.join(ops, "validate.mjs")], { encoding: "utf8" });
  if (v.status !== 0) { fs.writeFileSync(boardPath, boardRaw); return { ok: false, stderr: (v.stdout || "") + (v.stderr || "") }; }
  return { ok: true };
}
export function appendOpsLog(ops, entry) {
  fs.appendFileSync(path.join(ops, "log.jsonl"), JSON.stringify({ ts: new Date().toISOString(), ...entry }) + "\n");
}

// ── v11: wp_watch (resident watcher) shared helpers ─────────────────────────
export const WATCH_STALE_MS_DEFAULT = 15 * 60 * 1000; // 3 × 5-min cycles
export function watchPaths(ops) {
  const tmp = path.join(ops, ".tmp");
  return {
    tmp,
    heartbeat: path.join(tmp, "wp_watch.heartbeat.json"),
    log: path.join(tmp, "wp_watch.log"),
    state: path.join(tmp, "wp_watch.state.json"),
    pid: path.join(tmp, "wp_watch.pid"),
  };
}
/** Create .tmp and make sure it is git-ignored on its own ('*'). */
export function ensureTmpDir(ops) {
  const { tmp } = watchPaths(ops);
  fs.mkdirSync(tmp, { recursive: true });
  const gi = path.join(tmp, ".gitignore");
  if (!fs.existsSync(gi)) fs.writeFileSync(gi, "*\n");
  return tmp;
}
/** Atomic JSON write (tmp + rename) so readers never see half a file. */
export function writeJsonAtomic(p, obj) { writeFileDurable(p, JSON.stringify(obj, null, 2) + "\n"); } // v15: fsync + rename
export function readJsonOr(p, fallback) {
  try { return JSON.parse(fs.readFileSync(p, "utf8")); } catch { return fallback; }
}
/** Is pid alive and (on Linux) really a wp_watch process? Guards against pid reuse. */
export function isWatchPidAlive(pid) {
  if (!Number.isInteger(pid) || pid <= 0) return false;
  try { process.kill(pid, 0); } catch (e) { if (e.code !== "EPERM") return false; }
  try {
    const cmd = fs.readFileSync(`/proc/${pid}/cmdline`, "utf8");
    return cmd.includes("wp_watch.mjs");
  } catch { return true; }
}
/**
 * tick-side watcher health: heartbeat missing or older than staleMs → emit one
 * team_ops.watcher_down (dedup: skip while an unacked watcher_down exists whose ts is
 * newer than the last healthy heartbeat / exists at all when heartbeat is missing).
 */
export function checkWatcherAndEmit(ops, { now = new Date(), staleMs = WATCH_STALE_MS_DEFAULT, autostart = false } = {}) {
  const { heartbeat } = watchPaths(ops);
  const hb = readJsonOr(heartbeat, null);
  const hbTs = hb && Date.parse(hb.ts);
  const ageMs = Number.isFinite(hbTs) ? now.getTime() - hbTs : null;
  const healthy = ageMs !== null && ageMs <= staleMs;
  const res = {
    heartbeat: ".litecode/ops/.tmp/wp_watch.heartbeat.json",
    present: !!hb,
    pid: hb?.pid ?? null,
    supervisor_pid: hb?.supervisor_pid ?? null,
    last_heartbeat_at: hb?.ts ?? null,
    age_sec: ageMs === null ? null : Math.round(ageMs / 1000),
    stale_after_sec: Math.round(staleMs / 1000),
    healthy,
    emitted: null,
  };
  if (healthy) return res;
  // v15: auto-restart (tick) — runs `wp_watch.mjs start` with the caller's env, records team_ops.watcher_restarted
  if (autostart) {
    const r = spawnSync(process.execPath, [path.join(ops, "tools", "wp_watch.mjs"), "start"], { encoding: "utf8", env: process.env, cwd: path.join(ops, "..", ".."), timeout: 20000 });
    const out = (r.stdout || "").split("\n").filter((l) => l.startsWith("{")).pop();
    let st = null; try { st = JSON.parse(out); } catch {}
    const ok = r.status === 0 && !!st && st.running === true;
    res.autostart = { attempted: true, ok, supervisor_pid: st?.supervisor_pid ?? null, exit: r.status };
    const summary = `wp_watch was down (last heartbeat ${hb?.ts ?? "missing"}); ops.tick auto-restart ${ok ? "OK supervisor_pid=" + st.supervisor_pid : "FAILED exit=" + r.status}`;
    const { added } = mutateInbox(ops, (box) => {
      box.inbox.push({ id: newEventId("watcher-restarted", now.getTime()), ts: now.toISOString(), type: "team_ops.watcher_restarted", source: "ops.tick", summary, wp: null, acked: [],
        meta: { target: "gm", ok, last_heartbeat_at: hb?.ts ?? null, age_sec: res.age_sec, old_pid: hb?.pid ?? null, supervisor_pid: st?.supervisor_pid ?? null } });
    });
    res.emitted = { id: added[0].id, summary };
    if (ok) return res;
  }
  let ev = null;
  const summary = hb
    ? `wp_watch heartbeat stale: last ${hb.ts} (${Math.round(ageMs / 60000)}m ago, pid=${hb.pid}); restart: node .litecode/ops/tools/wp_watch.mjs start`
    : "wp_watch heartbeat missing (.litecode/ops/.tmp/wp_watch.heartbeat.json); start: node .litecode/ops/tools/wp_watch.mjs start";
  mutateInbox(ops, (box) => {
    const open = box.inbox.find((e) => e.type === "team_ops.watcher_down" && (!e.acked || e.acked.length === 0) && (!Number.isFinite(hbTs) || Date.parse(e.ts) >= hbTs));
    if (open) { res.already_open = open.id; return false; }
    ev = { id: newEventId("watcher", now.getTime()), ts: now.toISOString(), type: "team_ops.watcher_down", source: "ops.tick", summary, wp: null, acked: [],
      meta: { target: "gm", last_heartbeat_at: hb?.ts ?? null, age_sec: res.age_sec, pid: hb?.pid ?? null, last_error: hb?.last_error ?? null } };
    box.inbox.push(ev);
  });
  if (!ev) return res;
  res.emitted = res.emitted ? { ...res.emitted, down: { id: ev.id, summary } } : { id: ev.id, summary };
  return res;
}
// ── WP-189: org tree / reporting / charter ───────────────────────────────────
export function opsConfigOf(board) {
  const c = board.ops_config || {};
  return {
    principal: c.principal || "andy",
    intake_agent: c.intake_agent || "tools_steward",
    escalate_to_principal: c.escalate_to_principal === true,
  };
}

export function deriveReporting(board) {
  const nodes = new Map((board.org_nodes || []).map((n) => [n.id, n]));
  const cfg = opsConfigOf(board);
  const superiorMap = new Map();
  for (const agent of board.agents || []) {
    if (!agent.node || !nodes.has(agent.node)) continue;
    const node = nodes.get(agent.node);
    if (agent.id === node.lead) {
      if (!node.parent) superiorMap.set(agent.id, cfg.principal);
      else {
        const parent = nodes.get(node.parent);
        superiorMap.set(agent.id, parent ? parent.lead : null);
      }
    } else {
      superiorMap.set(agent.id, node.lead);
    }
  }
  const subMap = new Map();
  for (const ag of board.agents || []) subMap.set(ag.id, []);
  for (const [id, superior] of superiorMap) {
    if (superior && subMap.has(superior)) subMap.get(superior).push(id);
  }
  for (const list of subMap.values()) list.sort();
  return { sup: superiorMap, subs: subMap };
}

export function applyDerivedReporting(board) {
  const derived = deriveReporting(board);
  for (const agent of board.agents || []) {
    if (!agent.node) continue;
    agent.superior = derived.sup.get(agent.id) ?? null;
    agent.subordinates = derived.subs.get(agent.id) ?? [];
  }
  return derived;
}

export function derivedDrift(board) {
  const derived = deriveReporting(board);
  const problems = [];
  for (const agent of board.agents || []) {
    if (!agent.node) continue;
    const wantSup = derived.sup.has(agent.id) ? (derived.sup.get(agent.id) ?? null) : null;
    if (agent.superior !== undefined && (agent.superior ?? null) !== wantSup) {
      problems.push(`角色 ${agent.id} 的 superior 应为 ${wantSup ?? "空"}，当前是 ${agent.superior ?? "空"}`);
    }
    if (agent.subordinates !== undefined) {
      const wantSubs = derived.subs.get(agent.id) ?? [];
      const got = [...(agent.subordinates ?? [])].sort();
      if (JSON.stringify(got) !== JSON.stringify(wantSubs)) {
        problems.push(`角色 ${agent.id} 的 subordinates 与组织树不一致`);
      }
    }
  }
  return problems;
}

export function superiorOfAgent(board, id) {
  const cfg = opsConfigOf(board);
  if (!id || id === cfg.principal) return null;
  return deriveReporting(board).sup.get(id) ?? null;
}

function nodeChain(nodes, id) {
  const chain = [];
  const seen = new Set();
  let cur = nodes.get(id);
  while (cur && !seen.has(cur.id)) {
    seen.add(cur.id);
    chain.unshift(cur.id);
    cur = cur.parent ? nodes.get(cur.parent) : null;
  }
  return chain;
}

export function orgTreeRows(board) {
  const nodes = new Map((board.org_nodes || []).map((n) => [n.id, n]));
  const rows = [];
  const seen = new Set();
  const walk = (node, depth) => {
    if (seen.has(node.id)) return;
    seen.add(node.id);
    const members = (board.agents || []).filter((a) => a.node === node.id).map((a) => a.id).sort();
    rows.push({ depth, id: node.id, name: node.name, parent: node.parent ?? null, lead: node.lead, members, chain: nodeChain(nodes, node.id) });
    const kids = [...nodes.values()].filter((n) => n.parent === node.id).sort((a, b) => (a.id < b.id ? -1 : 1));
    for (const kid of kids) walk(kid, depth + 1);
  };
  for (const node of [...nodes.values()].filter((n) => !n.parent).sort((a, b) => (a.id < b.id ? -1 : 1))) walk(node, 0);
  for (const node of nodes.values()) if (!seen.has(node.id)) walk(node, 0);
  return rows;
}

export function orgProblems(board) {
  const problems = [];
  const nodes = board.org_nodes || [];
  if (!nodes.length) return problems;
  const nodeMap = new Map(nodes.map((n) => [n.id, n]));
  const agentIds = new Set((board.agents || []).map((a) => a.id));
  const cfg = opsConfigOf(board);
  if (cfg.intake_agent && !agentIds.has(cfg.intake_agent)) {
    problems.push(`ops_config.intake_agent 指向不存在的角色 ${cfg.intake_agent}`);
  }
  const leadOwner = new Map();
  for (const node of nodes) {
    if (node.parent) {
      if (node.parent === node.id) problems.push(`组织节点 ${node.id} 不能以自己为父节点`);
      else if (!nodeMap.has(node.parent)) problems.push(`组织节点 ${node.id} 的父节点 ${node.parent} 不存在`);
    }
    if (!node.lead) problems.push(`组织节点 ${node.id} 缺少 lead`);
    else {
      if (leadOwner.has(node.lead)) problems.push(`角色 ${node.lead} 同时是 ${leadOwner.get(node.lead)} 和 ${node.id} 的负责人`);
      else leadOwner.set(node.lead, node.id);
      if (!agentIds.has(node.lead)) problems.push(`组织节点 ${node.id} 的负责人 ${node.lead} 没有角色卡`);
      else {
        const lead = (board.agents || []).find((a) => a.id === node.lead);
        if (lead && lead.node && lead.node !== node.id) {
          problems.push(`负责人 ${node.lead} 的 node 是 ${lead.node}，不是 ${node.id}`);
        }
      }
    }
  }
  const seen = new Set();
  const visit = (id, stack) => {
    if (stack.has(id)) { problems.push(`组织树成环：${[...stack, id].join(" → ")}`); return; }
    if (seen.has(id)) return;
    seen.add(id); stack.add(id);
    for (const n of nodes) if (n.parent === id) visit(n.id, stack);
    stack.delete(id);
  };
  for (const n of nodes) if (!n.parent) visit(n.id, new Set());
  for (const n of nodes) if (!seen.has(n.id)) visit(n.id, new Set());

  for (const agent of board.agents || []) {
    if (agent.status === "fired") continue;
    if (!agent.node) problems.push(`角色 ${agent.id} 缺少 node（必须挂到组织树）`);
    else if (!nodeMap.has(agent.node)) problems.push(`角色 ${agent.id} 的节点 ${agent.node} 不存在`);
  }
  problems.push(...derivedDrift(board));
  return problems;
}

function lineList(arr) {
  if (!arr || !arr.length) return "（无）";
  return arr.map((x) => `- ${x}`).join("\n");
}

/** Render role charter markdown. */
export function charterOf(board, id, { constitutionText = null } = {}) {
  const agent = (board.agents || []).find((a) => a.id === id);
  if (!agent) return null;
  const node = (board.org_nodes || []).find((n) => n.id === agent.node);
  const constitution = (constitutionText && String(constitutionText).trim()) || "（还没有总章程）";
  const superior = agent.superior || "（无）";
  const markdown = [
    `# 岗位说明：${agent.name}（${agent.id}）`,
    "",
    `状态：${agent.status}`,
    `所属节点：${node ? node.name : (agent.node || "（未挂）")}（${agent.node || "-"}）`,
    `上级：${superior}`,
    `下级：${(agent.subordinates ?? []).join("、") || "（无）"}`,
    "",
    "## 职责（scope）",
    lineList(agent.scope),
    "",
    "## 工作地点（place）",
    agent.place || "（无）",
    "",
    "## 工具",
    lineList(agent.tools),
    "",
    "## 接受的交付（accept_tests）",
    lineList(agent.accept_tests),
    "",
    "## 不接受的交付（reject_tests）",
    lineList(agent.reject_tests),
    "",
    "## 总章程",
    constitution,
    "",
  ].join("\n");
  return { id: agent.id, markdown };
}

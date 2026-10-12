import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
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
  const rows = board.work_packages.map((w) => `| ${esc(w.id)} | ${esc(w.title)} | ${esc(w.status)} | ${esc(w.owner)} | ${esc(w.reviewer)} |`);
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
export const OPEN_CLOCK_STATUSES = new Set(["in_progress", "review"]);

export function applyStatusEnteredAt(wp, to, nowIso = new Date().toISOString()) {
  if (OPEN_CLOCK_STATUSES.has(to)) {
    wp.status_entered_at = nowIso;
  } else {
    wp.status_entered_at = null;
  }
  return wp.status_entered_at;
}

/**
 * Scan board for WPs open >1h in work hours; emit team_ops.wp_timeout if no unacked
 * event for this open stretch. Returns { stale, emitted, quiet, work_hours }.
 */
export function scanAndEmitWpTimeouts(ops, { force = false, now = new Date(), source = "ops.tick" } = {}) {
  const workHours = force || isWorkHoursHkt(now);
  const board = JSON.parse(fs.readFileSync(path.join(ops, "board.json"), "utf8"));
  const inboxPath = path.join(ops, "events", "inbox.json");
  const box = JSON.parse(fs.readFileSync(inboxPath, "utf8"));
  const hourMs = 60 * 60 * 1000;
  const stale = [];
  for (const w of board.work_packages) {
    if (!OPEN_CLOCK_STATUSES.has(w.status) || !w.status_entered_at) continue;
    const entered = Date.parse(w.status_entered_at);
    if (!Number.isFinite(entered)) continue;
    const ageMs = now.getTime() - entered;
    if (ageMs < hourMs) continue;
    stale.push({
      id: w.id,
      status: w.status,
      owner: w.owner,
      reviewer: w.reviewer,
      status_entered_at: w.status_entered_at,
      age_minutes: Math.floor(ageMs / 60000),
    });
  }
  if (!workHours || stale.length === 0) {
    return { work_hours: workHours, stale, emitted: [], quiet: true };
  }
  const emitted = [];
  for (const s of stale) {
    const already = box.inbox.find(
      (e) =>
        e.type === "team_ops.wp_timeout" &&
        e.wp === s.id &&
        (!e.acked || e.acked.length === 0) &&
        Date.parse(e.ts) >= Date.parse(s.status_entered_at)
    );
    if (already) continue;
    const summary = `${s.id} ${s.status} open ${s.age_minutes}m since ${s.status_entered_at} (work-hours >1h); owner=${s.owner ?? "-"} reviewer=${s.reviewer ?? "-"}`;
    const ev = {
      id: "E-" + Date.now() + "-" + s.id.replace(/\W/g, ""),
      ts: now.toISOString(),
      type: "team_ops.wp_timeout",
      source,
      summary,
      wp: s.id,
      acked: [],
      meta: { status: s.status, status_entered_at: s.status_entered_at, age_minutes: s.age_minutes, owner: s.owner, reviewer: s.reviewer },
    };
    box.inbox.push(ev);
    emitted.push(ev);
    // ensure unique ids if same ms
    now = new Date(now.getTime() + 1);
  }
  if (emitted.length) {
    box.updated_at = new Date().toISOString();
    fs.writeFileSync(inboxPath, JSON.stringify(box, null, 2) + "\n");
  }
  return { work_hours: workHours, stale, emitted, quiet: emitted.length === 0 };
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
export function writeJsonAtomic(p, obj) {
  const t = `${p}.${process.pid}.${Date.now()}.tmp`;
  fs.writeFileSync(t, JSON.stringify(obj, null, 2) + "\n");
  fs.renameSync(t, p);
}
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
export function checkWatcherAndEmit(ops, { now = new Date(), staleMs = WATCH_STALE_MS_DEFAULT } = {}) {
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
  const inboxPath = path.join(ops, "events", "inbox.json");
  const box = JSON.parse(fs.readFileSync(inboxPath, "utf8"));
  const open = box.inbox.find(
    (e) => e.type === "team_ops.watcher_down" && (!e.acked || e.acked.length === 0) &&
      (!Number.isFinite(hbTs) || Date.parse(e.ts) >= hbTs)
  );
  if (open) { res.already_open = open.id; return res; }
  const summary = hb
    ? `wp_watch heartbeat stale: last ${hb.ts} (${Math.round(ageMs / 60000)}m ago, pid=${hb.pid}); restart: node .litecode/ops/tools/wp_watch.mjs start`
    : "wp_watch heartbeat missing (.litecode/ops/.tmp/wp_watch.heartbeat.json); start: node .litecode/ops/tools/wp_watch.mjs start";
  const ev = {
    id: "E-" + now.getTime() + "-watcher",
    ts: now.toISOString(),
    type: "team_ops.watcher_down",
    source: "ops.tick",
    summary,
    wp: null,
    acked: [],
    meta: { last_heartbeat_at: hb?.ts ?? null, age_sec: res.age_sec, pid: hb?.pid ?? null, last_error: hb?.last_error ?? null },
  };
  box.inbox.push(ev);
  box.updated_at = new Date().toISOString();
  fs.writeFileSync(inboxPath, JSON.stringify(box, null, 2) + "\n");
  res.emitted = { id: ev.id, summary };
  return res;
}

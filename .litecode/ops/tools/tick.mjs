import fs from "node:fs";
import path from "node:path";
import { loadRegistry, printVersionHeader, opsRoot, recordTrace, renderWpStatusTable, OPEN_CLOCK_STATUSES, scanAndEmitWpTimeouts, checkWatcherAndEmit, WATCH_STALE_MS_DEFAULT } from "./_lib.mjs";
const { reg } = loadRegistry(import.meta.url);
printVersionHeader(reg, "ops.tick");
const ops = opsRoot(import.meta.url);
const boardPath = path.join(ops, "board.json");
const board = JSON.parse(fs.readFileSync(boardPath, "utf8"));
const demand = JSON.parse(fs.readFileSync(path.join(ops, "demand", "board.json"), "utf8"));
const args = process.argv.slice(2);
const wp = (() => { const i = args.indexOf("--wp"); return i >= 0 ? args[i + 1] : null; })();
const forceTimeout = args.includes("--force-timeout-scan");
// v11: --watcher-stale-min N (default 15 = 3 wp_watch cycles); --no-watcher-check skips the heartbeat check
const staleMin = (() => { const i = args.indexOf("--watcher-stale-min"); const v = i >= 0 ? Number(args[i + 1]) : NaN; return Number.isFinite(v) && v > 0 ? v : null; })();
const watcherCheck = !args.includes("--no-watcher-check");

// v9: backfill status_entered_at for open WPs that predate the field (grace = now).
let backfilled = 0;
const nowIso = new Date().toISOString();
for (const w of board.work_packages) {
  if (OPEN_CLOCK_STATUSES.has(w.status)) {
    if (w.status_entered_at === undefined || w.status_entered_at === null) {
      w.status_entered_at = nowIso;
      backfilled++;
    }
  } else if (w.status_entered_at !== null) {
    w.status_entered_at = null;
    backfilled++;
  }
}
if (backfilled) {
  fs.writeFileSync(boardPath, JSON.stringify(board, null, 2) + "\n");
}

const ready = board.work_packages.filter((w) => w.status === "ready" || w.status === "backlog");
const blocked = board.work_packages.filter((w) => w.status === "blocked");
const intake = demand.demands.filter((d) => ["intake", "refined", "in_pipeline"].includes(d.status));
const breakpoints = board.work_packages.filter((w) => w.progress && String(w.progress).includes("BREAK"));
const openClock = board.work_packages
  .filter((w) => OPEN_CLOCK_STATUSES.has(w.status))
  .map((w) => ({ id: w.id, status: w.status, status_entered_at: w.status_entered_at, reviewer: w.reviewer, owner: w.owner }));

const timeout = scanAndEmitWpTimeouts(ops, { force: forceTimeout });
const watcher = watcherCheck
  ? checkWatcherAndEmit(ops, { staleMs: staleMin ? staleMin * 60000 : WATCH_STALE_MS_DEFAULT })
  : { skipped: true };
const wpTable = renderWpStatusTable(ops);
recordTrace(ops, wp, "ops.tick", reg.tools_version);
console.log(JSON.stringify({
  tools_version: reg.tools_version,
  tick_at: new Date().toISOString(),
  ready_wp: ready.map((w) => w.id),
  blocked_wp: blocked.map((w) => w.id),
  demand_open: intake.map((d) => d.id),
  breakpoints: breakpoints.map((w) => ({ id: w.id, progress: w.progress })),
  open_clock_wp: openClock,
  wp_timeout: {
    work_hours: timeout.work_hours,
    stale: timeout.stale,
    emitted: timeout.emitted.map((e) => ({ id: e.id, wp: e.wp, summary: e.summary })),
    quiet: timeout.quiet,
  },
  watcher,
  team_ops_agents: board.agents.filter((a) => ["gm", "demand_officer", "tools_steward"].includes(a.id)).map((a) => ({ id: a.id, status: a.status })),
  board: ".litecode/ops/tools/board.md",
  board_md_wp_table: wpTable,
  rule: "WP timeout events (team_ops.wp_timeout) fire only in HKT work hours when an open WP exceeds 1h; GM tick consumes unacked ones (wp_watch also POSTs them to the GM callback). v11: heartbeat of wp_watch missing/older than 15m -> one team_ops.watcher_down (restart: node .litecode/ops/tools/wp_watch.mjs start). Quiet when none stale.",
}, null, 2));

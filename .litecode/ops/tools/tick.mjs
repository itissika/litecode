import fs from "node:fs";
import path from "node:path";
import { loadRegistry, printVersionHeader, opsRoot, recordTrace } from "./_lib.mjs";
const { reg } = loadRegistry(import.meta.url);
printVersionHeader(reg, "ops.tick");
const ops = opsRoot(import.meta.url);
const board = JSON.parse(fs.readFileSync(path.join(ops,"board.json"),"utf8"));
const demand = JSON.parse(fs.readFileSync(path.join(ops,"demand","board.json"),"utf8"));
const args=process.argv.slice(2); const wp=(()=>{const i=args.indexOf("--wp");return i>=0?args[i+1]:null})();
const ready = board.work_packages.filter(w => w.status==="ready" || w.status==="backlog");
const blocked = board.work_packages.filter(w => w.status==="blocked");
const intake = demand.demands.filter(d => ["intake","refined","in_pipeline"].includes(d.status));
const breakpoints = board.work_packages.filter(w => w.progress && String(w.progress).includes("BREAK"));
recordTrace(ops, wp, "ops.tick", reg.tools_version);
console.log(JSON.stringify({
  tools_version: reg.tools_version,
  tick_at: new Date().toISOString(),
  ready_wp: ready.map(w=>w.id),
  blocked_wp: blocked.map(w=>w.id),
  demand_open: intake.map(d=>d.id),
  breakpoints: breakpoints.map(w=>({id:w.id,progress:w.progress})),
  team_ops_agents: board.agents.filter(a=>["gm","demand_officer","tools_steward"].includes(a.id)).map(a=>({id:a.id,status:a.status})),
  board: ".litecode/ops/tools/board.md",
  rule: "Product bots and group stay unaware until Andy launch; no first business message to cold product agents"
}, null, 2));
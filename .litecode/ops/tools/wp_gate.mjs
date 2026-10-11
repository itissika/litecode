import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { loadRegistry, printVersionHeader, opsRoot, recordTrace } from "./_lib.mjs";
const { reg } = loadRegistry(import.meta.url);
printVersionHeader(reg, "ops.wp_gate");
const ops = opsRoot(import.meta.url);
const boardPath = path.join(ops,"board.json");
const boardRaw = fs.readFileSync(boardPath,"utf8");
const board = JSON.parse(boardRaw);
const tracesPath = path.join(ops,"tool_traces.jsonl");
const traces = fs.existsSync(tracesPath) ? fs.readFileSync(tracesPath,"utf8").trim().split(/\n/).filter(Boolean).map(l=>JSON.parse(l)) : [];
const args=process.argv.slice(2);
function arg(n){const i=args.indexOf(n);return i>=0?args[i+1]:null}
const id=arg("--id"), to=arg("--to"), source=arg("--source")||"tools_steward";
const dryRun=args.includes("--dry-run");
if(!id||!to){console.error("usage: --id WP-xxx --to <status> [--dry-run] [--source <agent_id>]");process.exit(2)}
const STATUSES=["backlog","ready","in_progress","review","done","blocked","cancelled"];
if(!STATUSES.includes(to)){console.error("invalid status",to);process.exit(2)}
const wp=board.work_packages.find(w=>w.id===id);
if(!wp){console.error("missing WP",id);process.exit(1)}
for(const d of wp.depends_on){
  const dep=board.work_packages.find(w=>w.id===d);
  if(!dep||dep.status!=="done"){console.error("dependency not done",d);process.exit(1)}
}
if(to==="blocked" && wp.blocker && wp.blocker.kind==="other"){
  console.error("other blocker cannot set status=blocked");process.exit(1)
}
if(to==="done"){
  const candidates=[
    wp.artifact_path,
    wp.artifact_path && path.join(ops, wp.artifact_path.replace(/^\.litecode\/ops\//,"")),
    wp.artifact_path && path.join(ops,"artifacts",path.basename(wp.artifact_path))
  ].filter(Boolean);
  if(!candidates.some(p=>fs.existsSync(p))){console.error("done requires existing artifact_path");process.exit(1)}
  if(traces.filter(t=>t.wp===id).length===0){console.error("done requires tool_traces for WP");process.exit(1)}
}
const from=wp.status;
const tools_used=traces.filter(t=>t.wp===id).length;
if(dryRun){
  recordTrace(ops, id, "ops.wp_gate", reg.tools_version);
  console.log(JSON.stringify({ ok:true, id, from, to, dry_run:true, written:false, tools_used, tools_version: reg.tools_version }));
  process.exit(0);
}
if(from===to){
  recordTrace(ops, id, "ops.wp_gate", reg.tools_version);
  console.log(JSON.stringify({ ok:true, id, from, to, written:false, note:"already at status", tools_used, tools_version: reg.tools_version }));
  process.exit(0);
}
wp.status=to;
fs.writeFileSync(boardPath, JSON.stringify(board,null,2)+(boardRaw.endsWith("\n")?"\n":""));
const v=spawnSync(process.execPath,[path.join(ops,"validate.mjs")],{encoding:"utf8"});
if(v.status!==0){
  fs.writeFileSync(boardPath, boardRaw);
  process.stderr.write(v.stdout||""); process.stderr.write(v.stderr||"");
  console.error("validate failed after write; board.json restored, status unchanged");
  process.exit(1);
}
fs.appendFileSync(path.join(ops,"log.jsonl"), JSON.stringify({ts:new Date().toISOString(),kind:"status_change",source,summary:`${id}: ${from} -> ${to} (ops.wp_gate)`,evidence:[".litecode/ops/board.json"]})+"\n");
recordTrace(ops, id, "ops.wp_gate", reg.tools_version);
console.log(JSON.stringify({ ok:true, id, from, to, written:true, validated:true, tools_used, tools_version: reg.tools_version }));

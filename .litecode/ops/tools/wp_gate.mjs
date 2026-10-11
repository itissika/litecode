import fs from "node:fs";
import path from "node:path";
import { loadRegistry, printVersionHeader, opsRoot, recordTrace } from "./_lib.mjs";
const { reg } = loadRegistry(import.meta.url);
printVersionHeader(reg, "ops.wp_gate");
const ops = opsRoot(import.meta.url);
const board = JSON.parse(fs.readFileSync(path.join(ops,"board.json"),"utf8"));
const tracesPath = path.join(ops,"tool_traces.jsonl");
const traces = fs.existsSync(tracesPath) ? fs.readFileSync(tracesPath,"utf8").trim().split(/\n/).filter(Boolean).map(l=>JSON.parse(l)) : [];
const args=process.argv.slice(2);
function arg(n){const i=args.indexOf(n);return i>=0?args[i+1]:null}
const id=arg("--id"), to=arg("--to");
if(!id||!to){console.error("usage: --id WP-xxx --to <status>");process.exit(2)}
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
recordTrace(ops, id, "ops.wp_gate", reg.tools_version);
console.log(JSON.stringify({ ok:true, id, to, tools_used: traces.filter(t=>t.wp===id).length, tools_version: reg.tools_version }));
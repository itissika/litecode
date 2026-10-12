import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { loadRegistry, printVersionHeader, opsRoot, recordTrace, renderWpStatusTable, applyStatusEnteredAt, WP_STATUSES, isNewModelWp, isApproved, approverOf, childrenOf, CLOSED_STATUSES } from "./_lib.mjs";
import { kbCheck, summarize, findingsFor } from "./kb_check.mjs";
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
if(!id||!to){console.error("usage: --id WP-xxx --to <status> [--dry-run] [--source <agent_id>] [--kb-nodes k1,k2]");process.exit(2)}
const STATUSES=WP_STATUSES; // v13: + drafting, awaiting_approval (prefer ops.wp for those)
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
// v13: nested-model WPs (parent/goal/history set). Old WPs skip this block entirely.
if(isNewModelWp(wp)){
  if(to==="in_progress" && !isApproved(wp)){console.error(`new-model WP ${id}: in_progress requires an approve in history (ops.wp approve by ${approverOf(wp)})`);process.exit(1)}
  if(to==="in_progress" && wp.parent){const par=board.work_packages.find(w=>w.id===wp.parent); if(par && isNewModelWp(par) && !isApproved(par)){console.error(`new-model WP ${id}: parent ${par.id} plan not approved yet; cannot start`);process.exit(1)}}
  if(to==="review"||to==="done"){
    const open=childrenOf(board,id).filter(c=>!CLOSED_STATUSES.has(c.status));
    if(open.length){console.error(`new-model WP ${id}: children not closed: ${open.map(c=>c.id+"("+c.status+")").join(", ")}`);process.exit(1)}
  }
  if(to==="done" && source!==approverOf(wp)){console.error(`new-model WP ${id}: only approver ${approverOf(wp)} may set done (--source ${source}); use ops.wp close`);process.exit(1)}
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
// v7: knowledge-type WP -> involved nodes must have no ops.kb_check errors before review/done.
// Knowledge-type = domain "knowledge", or the WP names nodes (knowledge/<path>.md, [@ key="k"], node:k
// in title/result/done_when/progress/artifact_path/handoff_path), or nodes carry `domain : <wp.domain>`,
// or --kb-nodes is given. Involved nodes = the union; a "knowledge" WP that names none checks all nodes.
let kbGate=null;
if(to==="review"||to==="done"){
  const text=[wp.title,wp.result,...(wp.done_when||[]),wp.progress||"",wp.artifact_path||"",wp.handoff_path||""].join("\n");
  const named=new Set();
  for(const m of text.matchAll(/knowledge\/[^\s`'"，。、；）)\]]+?\.md/g)) named.add(m[0]);
  for(const m of text.matchAll(/\[@ key="([^"]+)"\]/g)) named.add(m[1].trim());
  for(const m of text.matchAll(/\bnode:([A-Za-z0-9_][A-Za-z0-9_-]*)/g)) named.add(m[1]);
  const flag=arg("--kb-nodes"); if(flag) for(const k of flag.split(",").map(x=>x.trim()).filter(Boolean)) named.add(k);
  const domainLinked=wp.domain!=="team-ops";
  if(wp.domain==="knowledge"||named.size>0||domainLinked){
    const kb=kbCheck();
    let keys=[...named];
    if(domainLinked) for(const n of kb.nodes) if(n.domain===wp.domain) keys.push(n.path);
    const isKnowledgeWp=wp.domain==="knowledge"||keys.length>0;
    if(isKnowledgeWp){
      const sel=keys.length>0?findingsFor(kb,keys):{nodes:kb.nodes,findings:kb.findings,unknown:[]};
      const errs=sel.findings.filter(f=>f.severity==="error");
      kbGate={nodes:sel.nodes.length,errors:errs.length,warnings:sel.findings.length-errs.length,unknown:sel.unknown};
      if(sel.unknown.length){console.error("kb gate: WP names nodes that do not exist:",sel.unknown.join(", "));process.exit(1)}
      if(errs.length){
        console.error(`kb gate: ${errs.length} kb_check error(s) on involved nodes; fix before ${to}`);
        for(const f of errs.slice(0,20)) console.error(`  [E] ${f.code}  ${f.node??"-"}  ${f.path}${f.line?":"+f.line:""}  ${f.message}`);
        process.exit(1);
      }
    }
  }
}
const from=wp.status;
const tools_used=traces.filter(t=>t.wp===id).length;
if(dryRun){
  recordTrace(ops, id, "ops.wp_gate", reg.tools_version);
  console.log(JSON.stringify({ ok:true, id, from, to, dry_run:true, written:false, tools_used, tools_version: reg.tools_version, ...(kbGate?{kb_gate:kbGate}:{}) }));
  process.exit(0);
}
if(from===to){
  recordTrace(ops, id, "ops.wp_gate", reg.tools_version);
  console.log(JSON.stringify({ ok:true, id, from, to, written:false, note:"already at status", tools_used, tools_version: reg.tools_version }));
  process.exit(0);
}
wp.status=to;
applyStatusEnteredAt(wp, to);
fs.writeFileSync(boardPath, JSON.stringify(board,null,2)+(boardRaw.endsWith("\n")?"\n":""));
const v=spawnSync(process.execPath,[path.join(ops,"validate.mjs")],{encoding:"utf8"});
if(v.status!==0){
  fs.writeFileSync(boardPath, boardRaw);
  process.stderr.write(v.stdout||""); process.stderr.write(v.stderr||"");
  console.error("validate failed after write; board.json restored, status unchanged");
  process.exit(1);
}
fs.appendFileSync(path.join(ops,"log.jsonl"), JSON.stringify({ts:new Date().toISOString(),kind:"status_change",source,summary:`${id}: ${from} -> ${to} (ops.wp_gate)`,evidence:[".litecode/ops/board.json"]})+"\n");
const wpTable=renderWpStatusTable(ops);
recordTrace(ops, id, "ops.wp_gate", reg.tools_version);
console.log(JSON.stringify({ ok:true, id, from, to, written:true, board_md_wp_table:wpTable.rows, validated:true, tools_used, tools_version: reg.tools_version, ...(kbGate?{kb_gate:kbGate}:{}) }));

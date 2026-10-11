import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { loadRegistry, printVersionHeader, opsRoot, recordTrace } from "./_lib.mjs";
import { kbCheck, summarize } from "./kb_check.mjs";
const { reg } = loadRegistry(import.meta.url);
printVersionHeader(reg, "ops.validate_board");
const ops = opsRoot(import.meta.url);
const args=process.argv.slice(2); const wp=(()=>{const i=args.indexOf("--wp");return i>=0?args[i+1]:null})();
const r = spawnSync(process.execPath, [path.join(ops, "validate.mjs")], { encoding: "utf8" });
process.stdout.write(r.stdout||""); process.stderr.write(r.stderr||"");
if (r.status !== 0) process.exit(r.status||1);
const demand = JSON.parse(fs.readFileSync(path.join(ops,"demand","board.json"),"utf8"));
if (demand.schema_version !== 1) { console.error("demand schema_version"); process.exit(1); }
// v7: knowledge base must have no errors (ops.kb_check; warnings do not fail)
const kb = kbCheck();
const ks = summarize(kb.findings);
const kbSummary = { nodes: kb.meta.nodes, errors: ks.errors, warnings: ks.warnings, tool: "ops.kb_check" };
if (ks.errors > 0) {
  console.error(`INVALID: knowledge base has ${ks.errors} error(s) (ops.kb_check); details: node .litecode/ops/tools/kb_check.mjs`);
  for (const f of kb.findings.filter((x) => x.severity === "error").slice(0, 20)) console.error(`  [E] ${f.code}  ${f.node ?? "-"}  ${f.path}${f.line ? ":" + f.line : ""}  ${f.message}`);
  console.log(JSON.stringify({ ok:false, tools_version: reg.tools_version, kb: kbSummary }));
  process.exit(1);
}
recordTrace(ops, wp, "ops.validate_board", reg.tools_version);
console.log(JSON.stringify({ ok:true, tools_version: reg.tools_version, demands: demand.demands.length, kb: kbSummary, board: ".litecode/ops/tools/board.md" }));

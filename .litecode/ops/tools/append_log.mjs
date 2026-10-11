import fs from "node:fs";
import path from "node:path";
import { loadRegistry, printVersionHeader, opsRoot, recordTrace } from "./_lib.mjs";
const { reg } = loadRegistry(import.meta.url);
printVersionHeader(reg, "ops.append_log");
const ops = opsRoot(import.meta.url);
const args = process.argv.slice(2);
function arg(n){const i=args.indexOf(n);return i>=0?args[i+1]:null}
const kind=arg("--kind"), source=arg("--source"), summary=arg("--summary"), evidence=arg("--evidence"), wp=arg("--wp");
if(!kind||!source||!summary){console.error("usage: --kind --source --summary [--evidence] [--wp]");process.exit(2)}
fs.appendFileSync(path.join(ops,"log.jsonl"), JSON.stringify({ts:new Date().toISOString(),kind,source,summary,evidence:evidence?evidence.split(","):[]})+"\n");
recordTrace(ops, wp, "ops.append_log", reg.tools_version);
console.log(JSON.stringify({ ok:true, kind, wp: wp||null }));
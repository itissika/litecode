import fs from "node:fs";
import path from "node:path";
import { loadRegistry, printVersionHeader, opsRoot, recordTrace } from "./_lib.mjs";
const { reg } = loadRegistry(import.meta.url);
printVersionHeader(reg, "ops.emit_event");
const ops = opsRoot(import.meta.url);
const args=process.argv.slice(2);
function arg(n){const i=args.indexOf(n);return i>=0?args[i+1]:null}
const type=arg("--type"), source=arg("--source"), summary=arg("--summary"), wp=arg("--wp");
if(!type||!source||!summary){console.error("usage: --type --source --summary [--wp]");process.exit(2)}
const inboxPath=path.join(ops,"events","inbox.json");
const box=JSON.parse(fs.readFileSync(inboxPath,"utf8"));
const ev={id:"E-"+Date.now(),ts:new Date().toISOString(),type,source,summary,wp:wp||null,acked:[]};
box.inbox.push(ev); box.updated_at=new Date().toISOString();
fs.writeFileSync(inboxPath, JSON.stringify(box,null,2));
recordTrace(ops, wp, "ops.emit_event", reg.tools_version);
console.log(JSON.stringify({ok:true,event:ev}));
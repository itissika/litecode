import fs from "node:fs";
import path from "node:path";
import { loadRegistry, printVersionHeader, opsRoot, recordTrace } from "./_lib.mjs";
const { reg } = loadRegistry(import.meta.url);
printVersionHeader(reg, "ops.ack_event");
const ops = opsRoot(import.meta.url);
const args=process.argv.slice(2);
function arg(n){const i=args.indexOf(n);return i>=0?args[i+1]:null}
const id=arg("--id"), by=arg("--by"), wp=arg("--wp");
if(!id||!by){console.error("usage: --id E-... --by <agent_id>");process.exit(2)}
const inboxPath=path.join(ops,"events","inbox.json");
const box=JSON.parse(fs.readFileSync(inboxPath,"utf8"));
const ev=box.inbox.find(e=>e.id===id);
if(!ev){console.error("missing event");process.exit(1)}
if(!ev.acked.includes(by)) ev.acked.push(by);
box.updated_at=new Date().toISOString();
fs.writeFileSync(inboxPath, JSON.stringify(box,null,2));
recordTrace(ops, wp, "ops.ack_event", reg.tools_version);
console.log(JSON.stringify({ok:true,id,acked:ev.acked}));
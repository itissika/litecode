import fs from "node:fs";
import path from "node:path";
import { loadRegistry, printVersionHeader, opsRoot, recordTrace, mutateInbox } from "./_lib.mjs";
const { reg } = loadRegistry(import.meta.url);
printVersionHeader(reg, "ops.ack_event");
const ops = opsRoot(import.meta.url);
const args=process.argv.slice(2);
function arg(n){const i=args.indexOf(n);return i>=0?args[i+1]:null}
const id=arg("--id"), by=arg("--by"), wp=arg("--wp");
if(!id||!by){console.error("usage: --id E-... --by <agent_id>");process.exit(2)}
// v15: locked + durable write
let acked=null;
mutateInbox(ops,(box)=>{const ev=box.inbox.find(e=>e.id===id); if(!ev) return false; if(!ev.acked.includes(by)) ev.acked.push(by); acked=ev.acked;});
if(!acked){console.error(`missing event ${id}: not in events/inbox.json (since v15 every delivered event is fsynced before send; an id unknown here was never durably recorded — see WP-032.md §0.2b)`);process.exit(1)}
recordTrace(ops, wp, "ops.ack_event", reg.tools_version);
console.log(JSON.stringify({ok:true,id,acked}));
import fs from "node:fs";
import path from "node:path";
import { loadRegistry, printVersionHeader, opsRoot, recordTrace, mutateInbox, newEventId } from "./_lib.mjs";
const { reg } = loadRegistry(import.meta.url);
printVersionHeader(reg, "ops.emit_event");
const ops = opsRoot(import.meta.url);
const args=process.argv.slice(2);
function arg(n){const i=args.indexOf(n);return i>=0?args[i+1]:null}
const type=arg("--type"), source=arg("--source"), summary=arg("--summary"), wp=arg("--wp");
if(!type||!source||!summary){console.error("usage: --type --source --summary [--wp] [--target <agent>]");process.exit(2)}
// v15: locked + durable (fsync) write; deliverable types also go to events/outbox.jsonl before any send
const ev={id:newEventId(""),ts:new Date().toISOString(),type,source,summary,wp:wp||null,acked:[],...(arg("--target")?{meta:{target:arg("--target")}}:{})};
mutateInbox(ops,(box)=>{box.inbox.push(ev);});
recordTrace(ops, wp, "ops.emit_event", reg.tools_version);
console.log(JSON.stringify({ok:true,event:ev}));
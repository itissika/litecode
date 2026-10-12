// ops.issue_new (v1, tools_version 15, WP-032) — allocate the next ISSUE-NNN under a lock and create the file
// exclusively (never overwrites). Next id = max(existing files, git HEAD issues, issues/.reserved.json) + 1.
// Usage: node .litecode/ops/tools/issue_new.mjs --as <agent_id> --title T --domain D [--grade G] [--layer L]
//          [--owner O] [--wp WP-x] [--json-file F (merge extra fields)] [--dry-run]
import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { loadRegistry, printVersionHeader, opsRoot, recordTrace, withLock, writeFileDurable } from "./_lib.mjs";
const TOOL = "ops.issue_new";
const { reg } = loadRegistry(import.meta.url);
printVersionHeader(reg, TOOL);
const ops = opsRoot(import.meta.url);
const dir = path.join(ops, "issues");
const args = process.argv.slice(2);
const opt = (n) => { const i = args.indexOf(n); return i >= 0 ? args[i + 1] ?? null : null; };
const as = opt("--as"), title = opt("--title"), domain = opt("--domain");
if (!as || !title || !domain) { console.error("usage: issue_new.mjs --as <agent_id> --title T --domain D [--grade G] [--layer L] [--owner O] [--wp WP-x] [--json-file F] [--dry-run]"); process.exit(2); }
const num = (id) => { const m = /^ISSUE-(\d{3,})$/.exec(id || ""); return m ? Number(m[1]) : 0; };
export function usedNumbers(issuesDir) {
  const used = new Set();
  for (const f of fs.readdirSync(issuesDir)) { const n = num(f.replace(/\.json$/, "")); if (n) used.add(n); }
  try { for (const r of JSON.parse(fs.readFileSync(path.join(issuesDir, ".reserved.json"), "utf8")).reserved || []) { const n = num(r.id); if (n) used.add(n); } } catch {}
  const repo = path.resolve(issuesDir, "..", "..", "..");
  const g = spawnSync("git", ["-C", repo, "ls-tree", "--name-only", "HEAD", path.relative(repo, issuesDir) + "/"], { encoding: "utf8" });
  if (g.status === 0) for (const l of g.stdout.split("\n")) { const n = num(path.basename(l).replace(/\.json$/, "")); if (n) used.add(n); }
  return used;
}
let extra = {};
if (opt("--json-file")) extra = JSON.parse(fs.readFileSync(opt("--json-file"), "utf8"));
const res = withLock(path.join(dir, ".alloc.lock"), () => {
  const used = usedNumbers(dir);
  let n = Math.max(0, ...used) + 1;
  const id = "ISSUE-" + String(n).padStart(3, "0");
  const doc = { id, grade: opt("--grade") ?? null, layer: opt("--layer") ?? null, domain, core: false, core_nodes: [], reporter: as, owner: opt("--owner") ?? as, status: "open", blocks: false, title, ...extra,
    history: [{ ts: new Date().toISOString(), by: as, change: `ops.issue_new 分配${opt("--wp") ? "（" + opt("--wp") + "）" : ""}` }, ...((extra.history) || [])] };
  doc.id = id; // id is never taken from extra
  if (args.includes("--dry-run")) return { id, dry_run: true };
  const p = path.join(dir, id + ".json");
  const fd = fs.openSync(p, "wx"); // exclusive create: fails if the file exists (never overwrite)
  try { fs.writeSync(fd, JSON.stringify(doc, null, 2) + "\n"); fs.fsyncSync(fd); } finally { fs.closeSync(fd); }
  return { id, path: path.relative(path.join(ops, "..", ".."), p) };
});
recordTrace(ops, opt("--wp"), TOOL, reg.tools_version);
console.log(JSON.stringify({ ok: true, ...res, tools_version: reg.tools_version }));

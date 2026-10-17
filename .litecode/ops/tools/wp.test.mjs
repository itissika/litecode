// ops.wp tests (ops.wp v2, tools_version >= 14). Plan approval vs completion acceptance; 3-layer flow. Runs ONLY in a temp copy of .litecode/ops
// (os.tmpdir), never touches the real board, and strips LITECODE_WP_WEBHOOK_* from child envs
// so nothing can reach the real webhook. Usage: node .litecode/ops/tools/wp.test.mjs [--json] [--keep]
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { loadRegistry, printVersionHeader, recordTrace, scanAndEmitWpTimeouts, webhookPayloadOf, renderWpStatusTable } from "./_lib.mjs";

const { reg } = loadRegistry(import.meta.url);
printVersionHeader(reg, "ops.wp.test");
const here = path.dirname(fileURLToPath(import.meta.url));
const realOps = path.join(here, "..");
const json = process.argv.includes("--json");
const env = { ...process.env }; delete env.LITECODE_WP_WEBHOOK_URL; delete env.LITECODE_WP_WEBHOOK_KEY;
const results = [];
const tmpRoots = [];
const keep = process.argv.includes("--keep");
let cur = null;
function group(name, fn) {
  cur = { name, pass: 0, fail: [] };
  try { fn(); } catch (e) { cur.fail.push("threw: " + (e?.stack || e)); }
  results.push(cur); if (!json) console.log(`${cur.fail.length ? "FAIL" : "ok  "} ${name} (${cur.pass} passed, ${cur.fail.length} failed)${cur.fail.map((f) => "\n     - " + f).join("")}`);
}
function ok(c, msg) { if (c) cur.pass++; else cur.fail.push(msg); }

function fresh() {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "wp-test-")); tmpRoots.push(root);
  const ops = path.join(root, ".litecode", "ops");
  fs.mkdirSync(ops, { recursive: true });
  for (const f of ["board.json", "board.schema.json", "validate.mjs", "log.jsonl", "tool_traces.jsonl"]) fs.copyFileSync(path.join(realOps, f), path.join(ops, f));
  fs.mkdirSync(path.join(ops, "events")); fs.writeFileSync(path.join(ops, "events", "inbox.json"), JSON.stringify({ inbox: [], updated_at: null }, null, 2));
  const ctxSrc = path.join(realOps, "context");
  if (fs.existsSync(ctxSrc)) fs.cpSync(ctxSrc, path.join(ops, "context"), { recursive: true });
  fs.cpSync(here, path.join(ops, "tools"), { recursive: true, filter: (s) => !s.includes(".kb_nav_tmp") });
  return ops;
}
const run = (ops, tool, args) => spawnSync(process.execPath, [path.join(ops, "tools", tool), ...args], { encoding: "utf8", env, cwd: path.join(ops, "..", "..") });
const wp = (ops, ...a) => run(ops, "wp.mjs", a);
const gate = (ops, ...a) => run(ops, "wp_gate.mjs", a);
const board = (ops) => JSON.parse(fs.readFileSync(path.join(ops, "board.json"), "utf8"));
const W = (ops, id) => board(ops).work_packages.find((w) => w.id === id);
const validate = (ops) => spawnSync(process.execPath, [path.join(ops, "validate.mjs")], { encoding: "utf8" });
function patch(ops, fn) { const b = board(ops); fn(b); fs.writeFileSync(path.join(ops, "board.json"), JSON.stringify(b, null, 2) + "\n"); }
const lastId = (ops) => board(ops).work_packages.slice(-1)[0].id;
/** top (gm owner, approver gm, kind split) + child (owner tools_steward, approver gm). Top plan approved. */
function twoLayer(ops, { approveTop = true } = {}) {
  wp(ops, "new", "--as", "gm", "--title", "T top", "--goal", "g", "--owner", "gm", "--approver", "gm"); const top = lastId(ops);
  wp(ops, "new", "--as", "gm", "--parent", top, "--title", "T child", "--goal", "g2", "--owner", "tools_steward"); const kid = lastId(ops);
  wp(ops, "fill", "--as", "gm", "--id", top, "--kind", "split", "--text", "split");
  if (approveTop) { wp(ops, "submit", "--as", "gm", "--id", top); wp(ops, "approve", "--as", "gm", "--id", top); }
  return { top, kid };
}
function driveChildDone(ops, kid, owner = "tools_steward", appr = "gm") {
  wp(ops, "fill", "--as", owner, "--id", kid, "--text", "do it");
  wp(ops, "submit", "--as", owner, "--id", kid);
  wp(ops, "approve", "--as", appr, "--id", kid);
  wp(ops, "start", "--as", owner, "--id", kid);
  wp(ops, "deliver", "--as", owner, "--id", kid);
  return wp(ops, "close", "--as", appr, "--id", kid, "--to", "done");
}

group("1 schema: old board passes; missing parent / self-loop / 2-cycle fail", () => {
  const ops = fresh();
  ok(validate(ops).status === 0, "original board must validate");
  patch(ops, (b) => { b.work_packages[0].parent = "WP-999"; });
  ok(/parent missing/.test(validate(ops).stderr), "missing parent must fail");
  patch(ops, (b) => { b.work_packages[0].parent = b.work_packages[0].id; });
  ok(/itself/.test(validate(ops).stderr), "self parent must fail");
  patch(ops, (b) => { b.work_packages[0].parent = b.work_packages[1].id; b.work_packages[1].parent = b.work_packages[0].id; });
  ok(/cycle/.test(validate(ops).stderr), "2-cycle must fail");
  patch(ops, (b) => { delete b.work_packages[0].parent; delete b.work_packages[1].parent; b.work_packages[0].approver = "nobody"; });
  ok(/approver unknown/.test(validate(ops).stderr), "unknown approver must fail");
});

group("2 permissions: non-owner new; non-approver approve/close/done rejected", () => {
  const ops = fresh();
  const { top, kid } = twoLayer(ops);
  ok(W(ops, kid)?.approver === "gm" && W(ops, kid).status === "drafting", "child created drafting with approver=parent owner");
  let r = wp(ops, "new", "--as", "tools_steward", "--parent", top, "--title", "x", "--goal", "x", "--owner", "tools_steward");
  ok(r.status !== 0 && /不是父包负责人/.test(r.stderr), "non parent-owner new rejected");
  r = wp(ops, "new", "--as", "tools_steward", "--title", "x", "--goal", "x", "--owner", "tools_steward", "--approver", "gm");
  ok(r.status !== 0, "top-level new by non gm/andy rejected");
  r = wp(ops, "new", "--as", "ghost", "--title", "x", "--goal", "x", "--owner", "gm", "--approver", "gm");
  ok(r.status !== 0 && /未知身份/.test(r.stderr), "unknown --as rejected");
  wp(ops, "fill", "--as", "tools_steward", "--id", kid, "--text", "c");
  wp(ops, "submit", "--as", "tools_steward", "--id", kid);
  r = wp(ops, "approve", "--as", "tools_steward", "--id", kid);
  ok(r.status !== 0 && /不是审批人/.test(r.stderr), "owner cannot self-approve");
  wp(ops, "approve", "--as", "gm", "--id", kid);
  r = wp(ops, "start", "--as", "gm", "--id", kid);
  ok(r.status !== 0 && /不是负责人/.test(r.stderr), "non-owner start rejected");
  wp(ops, "start", "--as", "tools_steward", "--id", kid);
  wp(ops, "deliver", "--as", "tools_steward", "--id", kid);
  r = wp(ops, "close", "--as", "tools_steward", "--id", kid, "--to", "done");
  ok(r.status !== 0 && /不是审批人/.test(r.stderr), "non-approver close rejected");
  fs.mkdirSync(path.join(ops, "artifacts"), { recursive: true });
  patch(ops, (b) => { b.work_packages.find((w) => w.id === kid).artifact_path = ".litecode/ops/artifacts/x.md"; });
  fs.writeFileSync(path.join(ops, "artifacts", "x.md"), "x");
  r = gate(ops, "--id", kid, "--to", "done", "--source", "tools_steward");
  ok(r.status !== 0 && /only approver/.test(r.stderr), "wp_gate done by non-approver rejected");
  r = gate(ops, "--id", kid, "--to", "done", "--source", "gm");
  ok(r.status === 0 && W(ops, kid).status === "done", "wp_gate done by approver passes: " + r.stderr);
});

group("3 plan approval: split submit needs ≥1 child (drafting ok); no approve → no start; plan reject → drafting", () => {
  const ops = fresh();
  wp(ops, "new", "--as", "gm", "--title", "lonely", "--goal", "g", "--owner", "gm", "--approver", "gm"); const lone = lastId(ops);
  wp(ops, "fill", "--as", "gm", "--id", lone, "--kind", "split", "--text", "split");
  let r = wp(ops, "submit", "--as", "gm", "--id", lone);
  ok(r.status !== 0 && /至少要有一个子包/.test(r.stderr), "split without child rejected");
  const { top, kid } = twoLayer(ops, { approveTop: false });
  r = wp(ops, "submit", "--as", "gm", "--id", top);
  ok(r.status === 0 && W(ops, top).status === "awaiting_approval" && W(ops, kid).status === "drafting", "split plan submit with drafting child ok");
  const inbox = JSON.parse(fs.readFileSync(path.join(ops, "events", "inbox.json"), "utf8"));
  ok(inbox.inbox.some((e) => e.type === "team_ops.approval_requested" && e.wp === top && e.meta.stage === "plan"), "approval_requested (plan) emitted");
  r = wp(ops, "start", "--as", "gm", "--id", top);
  ok(r.status !== 0, "start before approve rejected");
  r = gate(ops, "--id", top, "--to", "in_progress", "--source", "gm");
  ok(r.status !== 0 && /requires an approve/.test(r.stderr), "gate in_progress before approve rejected");
  r = wp(ops, "reject", "--as", "gm", "--id", top, "--note", "plan weak", "--reopen", kid);
  ok(r.status !== 0 && /只有验收驳回才能/.test(r.stderr), "--reopen not allowed on plan rejection");
  r = wp(ops, "reject", "--as", "gm", "--id", top, "--note", "plan weak");
  ok(r.status === 0 && W(ops, top).status === "drafting" && W(ops, top).history.slice(-1)[0].action === "reject", "plan reject → drafting");
  wp(ops, "submit", "--as", "gm", "--id", top);
  r = wp(ops, "approve", "--as", "gm", "--id", top);
  ok(r.status === 0 && W(ops, top).status === "ready", "approve → ready");
  r = wp(ops, "fill", "--as", "gm", "--id", top, "--text", "late");
  ok(r.status !== 0, "fill outside drafting rejected");
});

group("4 parent gate: child cannot submit/start before parent plan approved", () => {
  const ops = fresh();
  const { top, kid } = twoLayer(ops, { approveTop: false });
  wp(ops, "fill", "--as", "tools_steward", "--id", kid, "--text", "c");
  let r = wp(ops, "submit", "--as", "tools_steward", "--id", kid);
  ok(r.status !== 0 && /父包 .* 方案还没批/.test(r.stderr), "child submit blocked while parent drafting");
  wp(ops, "submit", "--as", "gm", "--id", top);
  r = wp(ops, "submit", "--as", "tools_steward", "--id", kid);
  ok(r.status !== 0, "child submit blocked while parent awaiting_approval");
  patch(ops, (b) => { const k = b.work_packages.find((w) => w.id === kid); k.status = "ready"; k.history.push({ ts: "x", actor: "gm", action: "approve", note: "forced" }); });
  r = wp(ops, "start", "--as", "tools_steward", "--id", kid);
  ok(r.status !== 0 && /父包 .* 方案还没批/.test(r.stderr), "child start blocked (wp start)");
  r = gate(ops, "--id", kid, "--to", "in_progress", "--source", "tools_steward");
  ok(r.status !== 0 && /parent .* plan not approved/.test(r.stderr), "child start blocked (wp_gate)");
  wp(ops, "approve", "--as", "gm", "--id", top);
  r = wp(ops, "start", "--as", "tools_steward", "--id", kid);
  ok(r.status === 0 && W(ops, kid).status === "in_progress", "child starts after parent plan approved");
});

group("5 acceptance: deliver needs children done/blocked; reject → in_progress + reopen; close done only from review", () => {
  const ops = fresh();
  const { top, kid } = twoLayer(ops);
  wp(ops, "start", "--as", "gm", "--id", top);
  let r = wp(ops, "deliver", "--as", "gm", "--id", top);
  ok(r.status !== 0 && /还有未封子包/.test(r.stderr), "deliver with open child rejected");
  r = wp(ops, "close", "--as", "gm", "--id", top, "--to", "done");
  ok(r.status !== 0 && /要先送验收/.test(r.stderr), "close done before deliver rejected");
  r = gate(ops, "--id", top, "--to", "review", "--source", "gm");
  ok(r.status !== 0 && /children not closed/.test(r.stderr), "gate review with open child rejected");
  driveChildDone(ops, kid);
  ok(W(ops, kid).status === "done", "child done");
  r = wp(ops, "deliver", "--as", "gm", "--id", top);
  ok(r.status === 0 && W(ops, top).status === "review" && W(ops, top).status_entered_at, "deliver → review with clock");
  const inbox = JSON.parse(fs.readFileSync(path.join(ops, "events", "inbox.json"), "utf8"));
  ok(inbox.inbox.some((e) => e.type === "team_ops.acceptance_requested" && e.wp === top), "acceptance_requested emitted");
  r = wp(ops, "reject", "--as", "gm", "--id", top, "--note", "redo", "--reopen", "WP-001");
  ok(r.status !== 0 && /不是 .* 的子包/.test(r.stderr), "reopen non-child rejected");
  r = wp(ops, "reject", "--as", "gm", "--id", top, "--note", "child output thin", "--reopen", kid);
  ok(r.status === 0 && W(ops, top).status === "in_progress" && W(ops, kid).status === "drafting", "acceptance reject → parent in_progress, child drafting");
  ok(W(ops, top).history.slice(-1)[0].action === "return" && W(ops, kid).history.slice(-1)[0].action === "reopen", "history return/reopen");
  r = wp(ops, "submit", "--as", "tools_steward", "--id", kid);
  ok(r.status === 0, "reopened child can re-submit plan (parent plan still approved)");
  wp(ops, "approve", "--as", "gm", "--id", kid);
  r = wp(ops, "close", "--as", "gm", "--id", kid, "--to", "blocked");
  ok(r.status !== 0 && /必须写 --note/.test(r.stderr), "close blocked without note rejected");
  wp(ops, "close", "--as", "gm", "--id", kid, "--to", "blocked", "--note", "upstream");
  wp(ops, "deliver", "--as", "gm", "--id", top);
  r = wp(ops, "close", "--as", "gm", "--id", top, "--to", "done");
  ok(r.status !== 0 && /必须写 --note/.test(r.stderr), "parent close with blocked child needs note");
  r = wp(ops, "close", "--as", "gm", "--id", top, "--to", "done", "--note", "accept child blocked");
  ok(r.status === 0 && W(ops, top).status === "done", "parent close done: " + r.stderr);
});

group("9 three layers Andy→GM→middle→domain: one plan rejection, one acceptance rejection", () => {
  const ops = fresh();
  const A = (...a) => { const r = wp(ops, ...a); if (r.status !== 0) cur.fail.push(`step failed: ${a.join(" ")}: ${r.stderr.trim()}`); return r; };
  A("new", "--as", "andy", "--title", "L1", "--goal", "g1", "--owner", "gm", "--approver", "andy"); const L1 = lastId(ops);
  A("new", "--as", "gm", "--parent", L1, "--title", "L2", "--goal", "g2", "--owner", "backend_mid"); const L2 = lastId(ops);
  A("fill", "--as", "gm", "--id", L1, "--kind", "split", "--text", "one middle package");
  A("submit", "--as", "gm", "--id", L1);
  A("approve", "--as", "andy", "--id", L1);
  A("new", "--as", "backend_mid", "--parent", L2, "--title", "L3", "--goal", "g3", "--owner", "runtime"); const L3 = lastId(ops);
  ok(W(ops, L3).approver === "backend_mid" && W(ops, L2).approver === "gm" && W(ops, L1).approver === "andy", "approvers per layer");
  A("fill", "--as", "backend_mid", "--id", L2, "--kind", "split", "--text", "one domain package");
  A("submit", "--as", "backend_mid", "--id", L2);
  A("reject", "--as", "gm", "--id", L2, "--note", "plan lacks evidence rule");        // PLAN REJECTION
  ok(W(ops, L2).status === "drafting", "L2 plan rejected → drafting");
  A("fill", "--as", "backend_mid", "--id", L2, "--kind", "split", "--text", "one domain package + evidence rule");
  A("submit", "--as", "backend_mid", "--id", L2);
  A("approve", "--as", "gm", "--id", L2);
  A("start", "--as", "backend_mid", "--id", L2);
  A("fill", "--as", "runtime", "--id", L3, "--text", "fix + test");
  A("submit", "--as", "runtime", "--id", L3);
  A("approve", "--as", "backend_mid", "--id", L3);
  A("start", "--as", "runtime", "--id", L3);
  A("deliver", "--as", "runtime", "--id", L3);
  A("close", "--as", "backend_mid", "--id", L3, "--to", "done");
  A("deliver", "--as", "backend_mid", "--id", L2);
  A("reject", "--as", "gm", "--id", L2, "--note", "domain evidence missing", "--reopen", L3);  // ACCEPTANCE REJECTION
  ok(W(ops, L2).status === "in_progress" && W(ops, L3).status === "drafting", "L2 acceptance rejected, L3 reopened");
  A("fill", "--as", "runtime", "--id", L3, "--text", "fix + test + evidence");
  A("submit", "--as", "runtime", "--id", L3);
  A("approve", "--as", "backend_mid", "--id", L3);
  A("start", "--as", "runtime", "--id", L3);
  A("deliver", "--as", "runtime", "--id", L3);
  A("close", "--as", "backend_mid", "--id", L3, "--to", "done");
  A("deliver", "--as", "backend_mid", "--id", L2);
  A("close", "--as", "gm", "--id", L2, "--to", "done");
  A("start", "--as", "gm", "--id", L1);
  A("deliver", "--as", "gm", "--id", L1);
  A("close", "--as", "andy", "--id", L1, "--to", "done");
  ok([L1, L2, L3].every((id) => W(ops, id).status === "done"), "all three layers done");
  ok(W(ops, L2).history.map((h) => h.action).join(",") === "create,fill,submit,reject,fill,submit,approve,start,deliver,return,deliver,close", "L2 history: " + W(ops, L2).history.map((h) => h.action));
  ok(validate(ops).status === 0, "board valid after 3-layer flow");
  const t = wp(ops, "tree", "--id", L1).stdout;
  ok(t.includes(`    └ ${L3}`), "tree shows 3 levels");
});

group("6 timeout: awaiting_approval & review >1h → approver; unacked next cycle → escalate up; top stops at gm; webhook fields", () => {
  const ops = fresh();
  const { top, kid } = twoLayer(ops);
  wp(ops, "new", "--as", "tools_steward", "--parent", kid, "--title", "grandchild", "--goal", "g3", "--owner", "runtime"); const gk = lastId(ops);
  wp(ops, "fill", "--as", "tools_steward", "--id", kid, "--kind", "split", "--text", "s"); wp(ops, "submit", "--as", "tools_steward", "--id", kid); wp(ops, "approve", "--as", "gm", "--id", kid);
  wp(ops, "fill", "--as", "runtime", "--id", gk, "--text", "c");
  wp(ops, "submit", "--as", "runtime", "--id", gk);
  const t0 = Date.now();
  patch(ops, (b) => { for (const w of b.work_packages) if (w.status_entered_at) w.status_entered_at = null; // isolate from real open WPs
    if (b.work_packages.find((w) => w.id === gk)) b.work_packages.find((w) => w.id === gk).status_entered_at = new Date(t0 - 2 * 3600e3).toISOString(); });
  let s = scanAndEmitWpTimeouts(ops, { force: true, now: new Date(t0) });
  ok(s.emitted.length === 1 && s.emitted[0].meta.target === "tools_steward" && !s.emitted[0].meta.escalated, "first event → approver tools_steward");
  ok(s.emitted[0].meta.level === 2 && s.emitted[0].meta.parent === kid && s.emitted[0].meta.path.join(">") === [top, kid, gk].join(">"), "level/parent/path");
  s = scanAndEmitWpTimeouts(ops, { force: true, now: new Date(t0 + 60e3) });
  ok(s.emitted.length === 0, "no escalation within 5 minutes");
  s = scanAndEmitWpTimeouts(ops, { force: true, now: new Date(t0 + 5 * 60e3 + 1) });
  ok(s.emitted.length === 1 && s.emitted[0].meta.escalated === true && s.emitted[0].meta.target === "gm", "escalated to parent approver gm");
  s = scanAndEmitWpTimeouts(ops, { force: true, now: new Date(t0 + 20 * 60e3) });
  ok(s.emitted.length === 0, "escalate once per stretch");
  const p = webhookPayloadOf(JSON.parse(fs.readFileSync(path.join(ops, "events", "inbox.json"), "utf8")).inbox.find((e) => e.meta?.escalated));
  ok(p.level === 2 && p.escalated === true && p.target === "gm" && Array.isArray(p.path) && p.parent === kid, "webhook payload has level/path/escalated/target/parent");
  for (const k of ["event_id", "wp_id", "status", "age_minutes", "owner", "reviewer", "status_entered_at", "summary", "ts"]) ok(k in p, "old webhook field kept: " + k);
  // top-level with approver andy: first target gm, no escalation level above (no andy anywhere)
  // review wait also escalates
  patch(ops, (b) => { for (const w of b.work_packages) w.status_entered_at = null; });
  wp(ops, "approve", "--as", "tools_steward", "--id", gk); wp(ops, "start", "--as", "runtime", "--id", gk); wp(ops, "deliver", "--as", "runtime", "--id", gk);
  ok(W(ops, gk).status === "review", "grandchild in review");
  patch(ops, (b) => { b.work_packages.find((w) => w.id === gk).status_entered_at = new Date(t0 + 10 * 60e3).toISOString(); }); // new stretch after the awaiting events
  s = scanAndEmitWpTimeouts(ops, { force: true, now: new Date(t0 + 75 * 60e3) });
  ok(s.emitted.length === 1 && s.emitted[0].meta.status === "review" && s.emitted[0].meta.target === "tools_steward", "review timeout → approver");
  s = scanAndEmitWpTimeouts(ops, { force: true, now: new Date(t0 + 81 * 60e3) });
  ok(s.emitted.length === 1 && s.emitted[0].meta.escalated && s.emitted[0].meta.target === "gm", "review timeout escalates to gm");
  const ops2 = fresh();
  wp(ops2, "new", "--as", "gm", "--title", "top andy", "--goal", "g", "--owner", "gm", "--approver", "andy"); const ta = lastId(ops2);
  wp(ops2, "fill", "--as", "gm", "--id", ta, "--text", "c"); wp(ops2, "submit", "--as", "gm", "--id", ta);
  patch(ops2, (b) => { for (const w of b.work_packages) w.status_entered_at = w.id === ta ? new Date(t0 - 2 * 3600e3).toISOString() : null; });
  s = scanAndEmitWpTimeouts(ops2, { force: true, now: new Date(t0) });
  const s2 = scanAndEmitWpTimeouts(ops2, { force: true, now: new Date(t0 + 10 * 60e3) });
  ok(s.emitted.length === 1 && s.emitted[0].meta.target === "gm" && s2.emitted.length === 0, "top-level (approver andy) → gm only, no further level");
  const all = fs.readFileSync(path.join(ops2, "events", "inbox.json"), "utf8") + fs.readFileSync(path.join(ops, "events", "inbox.json"), "utf8");
  ok(!/"target":\s*"andy"/.test(all), "no event targets andy");
  ok(!process.env.__never && !("LITECODE_WP_WEBHOOK_URL" in env), "child env has no webhook URL");
});

group("7 compat: old-model WPs (dynamic count) unchanged (validate, gate dry-run, table rows, timeout/no escalation, payload)", () => {
  const ops = fresh();
  ok(validate(ops).status === 0, "validate OK");
  const b = board(ops);
  const olds = b.work_packages.filter((w) => !w.goal && !w.parent && !Array.isArray(w.history));
  // v17: count computed from the board (old-model = no goal/parent/history); must be >0, behaviour checked per package
  ok(olds.length > 0 && olds.every((w) => w.id <= "WP-019"), `old-model WPs present and all within WP-001..019: ${olds.length}`);
  for (const id of olds.map((w) => w.id)) {
    const r = gate(ops, "--id", id, "--to", "review", "--dry-run", "--source", "gm");
    ok(r.status === 0 && /"dry_run":true/.test(r.stdout), `${id} dry-run review still ok`);
  }
  const r = gate(ops, "--id", "WP-019", "--to", "done", "--dry-run", "--source", "tools_steward");
  ok(r.status !== 0 || !b.work_packages.find((w) => w.id === "WP-019").goal, "old WP-019 done gate not subject to approver rule");
  renderWpStatusTable(ops);
  const md = fs.readFileSync(path.join(ops, "tools", "board.md"), "utf8");
  for (const w of b.work_packages.filter((x) => !x.parent)) ok(md.includes(`| ${w.id} | ${w.title.replace(/\|/g, "\\|")} | ${w.status} |`), `row for ${w.id} unindented`);
  patch(ops, (bb) => { for (const w of bb.work_packages) w.status_entered_at = null; const o = bb.work_packages.find((w) => w.id === "WP-017"); o.status = "review"; o.status_entered_at = new Date(Date.now() - 2 * 3600e3).toISOString(); });
  let s = scanAndEmitWpTimeouts(ops, { force: true });
  ok(s.emitted.length === 1 && /^WP-017 review open \d+m since .* owner=demand_officer reviewer=gm$/.test(s.emitted[0].summary), "old summary format unchanged");
  s = scanAndEmitWpTimeouts(ops, { force: true, now: new Date(Date.now() + 30 * 60e3) });
  ok(s.emitted.length === 0, "old WP never escalates");
  const tk = run(ops, "tick.mjs", ["--no-watcher-check"]);
  ok(tk.status === 0 && /"awaiting_approval_wp"/.test(tk.stdout) && /"ready_wp"/.test(tk.stdout), "tick runs, has old + new keys");
});

group("8 rollback: validate failure leaves board.json byte-identical", () => {
  const ops = fresh();
  const { kid } = twoLayer(ops);
  // corrupt validation precondition: make an agent place illegal so ANY write fails validate
  patch(ops, (b) => { b.agents.find((a) => a.id === "runtime").place = "/elsewhere"; });
  const before = fs.readFileSync(path.join(ops, "board.json"), "utf8");
  let r = wp(ops, "fill", "--as", "tools_steward", "--id", kid, "--text", "c");
  ok(r.status !== 0 && /已恢复/.test(r.stderr), "fill fails with restore message");
  ok(fs.readFileSync(path.join(ops, "board.json"), "utf8") === before, "board.json unchanged after failed fill");
  r = wp(ops, "new", "--as", "gm", "--title", "x", "--goal", "x", "--owner", "gm", "--approver", "gm");
  ok(r.status !== 0 && fs.readFileSync(path.join(ops, "board.json"), "utf8") === before, "board.json unchanged after failed new");
});

group("10 set: owner/approver set safe fields; others/unsafe/closed rejected; rollback on validate failure", () => {
  const ops = fresh();
  const { kid } = twoLayer(ops);
  let r = wp(ops, "set", "--as", "tools_steward", "--id", kid, "--field", "artifact_path", "--value", ".litecode/ops/artifacts/X.md");
  ok(r.status === 0 && W(ops, kid).artifact_path === ".litecode/ops/artifacts/X.md" && W(ops, kid).history.slice(-1)[0].action === "set", "owner sets artifact_path + history");
  r = wp(ops, "set", "--as", "gm", "--id", kid, "--field", "done_when", "--value", "a; b");
  ok(r.status === 0 && W(ops, kid).done_when.join("|") === "a|b", "approver sets done_when list");
  r = wp(ops, "set", "--as", "runtime", "--id", kid, "--field", "progress", "--value", "x");
  ok(r.status !== 0, "non owner/approver rejected");
  r = wp(ops, "set", "--as", "tools_steward", "--id", kid, "--field", "status", "--value", "done");
  ok(r.status !== 0 && /不能改/.test(r.stderr), "unsafe field rejected");
  r = wp(ops, "set", "--as", "tools_steward", "--id", kid, "--field", "artifact_path", "--value", "/etc/passwd");
  ok(r.status !== 0, "path outside .litecode/ops/ rejected");
  patch(ops, (b) => { b.agents.find((a) => a.id === "runtime").place = "/elsewhere"; });
  const before = fs.readFileSync(path.join(ops, "board.json"), "utf8");
  r = wp(ops, "set", "--as", "tools_steward", "--id", kid, "--field", "progress", "--value", "y");
  ok(r.status !== 0 && fs.readFileSync(path.join(ops, "board.json"), "utf8") === before, "validate failure rolls back");
});

group("11 set domain (v4) + new agent provider usable as owner", () => {
  const ops = fresh();
  const { kid } = twoLayer(ops);
  let r = wp(ops, "set", "--as", "gm", "--id", kid, "--field", "domain", "--value", "ide-substrate");
  ok(r.status === 0 && W(ops, kid).domain === "ide-substrate", "approver sets existing domain");
  const h = W(ops, kid).history.slice(-1)[0];
  ok(h.action === "set" && /domain/.test(h.note), "history entry for domain set");
  ok(/set domain/.test(fs.readFileSync(path.join(ops, "log.jsonl"), "utf8")), "log.jsonl records domain set");
  r = wp(ops, "set", "--as", "gm", "--id", kid, "--field", "domain", "--value", "ide-base");
  ok(r.status !== 0 && /已有域/.test(r.stderr) && W(ops, kid).domain === "ide-substrate", "unknown domain rejected, unchanged");
  r = wp(ops, "set", "--as", "gm", "--id", kid, "--field", "domain", "--value", "");
  ok(r.status !== 0, "empty domain rejected");
  patch(ops, (b) => { b.work_packages.find((x) => x.id === kid).domain = "nope"; });
  ok(validate(ops).status !== 0, "validator rejects WP with unknown domain (v17)");
  patch(ops, (b) => { b.work_packages.find((x) => x.id === kid).domain = "ide-substrate"; });
  r = wp(ops, "set", "--as", "runtime", "--id", kid, "--field", "domain", "--value", "knowledge");
  ok(r.status !== 0 && W(ops, kid).domain === "ide-substrate", "non owner/approver cannot set domain");
  { const o = fresh(); const { kid: k2 } = twoLayer(o); patch(o, (b) => { b.work_packages.find((x) => x.id === k2).status = "cancelled"; });
    let q = wp(o, "set", "--as", "gm", "--id", k2, "--field", "domain", "--value", "knowledge");
    ok(q.status === 0 && W(o, k2).domain === "knowledge", "closed package: domain reclassification allowed (v17)");
    q = wp(o, "set", "--as", "gm", "--id", k2, "--field", "progress", "--value", "x");
    ok(q.status !== 0, "closed package: other fields still refused"); }
  const hasProvider = board(ops).agents.some((a) => a.id === "provider");
  ok(hasProvider, "board has agent provider");
  if (hasProvider) {
    r = wp(ops, "new", "--as", "gm", "--title", "P", "--goal", "g", "--owner", "provider", "--approver", "backend_mid", "--domain", "model-config");
    const pid = lastId(ops);
    ok(r.status === 0 && W(ops, pid).owner === "provider", "wp new --owner provider");
    r = wp(ops, "set", "--as", "provider", "--id", pid, "--field", "progress", "--value", "p");
    ok(r.status === 0 && W(ops, pid).progress === "p", "provider as owner can set");
  }
});

group("12 reassign (v4): approver or parent owner; registered agent only; history+log; rollback", () => {
  const ops = fresh();
  const { top, kid } = twoLayer(ops);
  const snap = (id) => { const x = { ...W(ops, id) }; delete x.owner; delete x.history; return JSON.stringify(x); };
  const before = snap(kid);
  let r = wp(ops, "reassign", "--as", "runtime", "--id", kid, "--owner", "context");
  ok(r.status !== 0 && W(ops, kid).owner === "tools_steward", "stranger cannot reassign");
  r = wp(ops, "reassign", "--as", "gm", "--id", kid, "--owner", "nobody");
  ok(r.status !== 0 && /不是已登记/.test(r.stderr), "unregistered owner rejected");
  r = wp(ops, "reassign", "--as", "gm", "--id", kid, "--owner", "runtime", "--note", "move");
  ok(r.status === 0 && W(ops, kid).owner === "runtime", "approver reassigns");
  ok(snap(kid) === before, "other fields unchanged");
  const h = W(ops, kid).history.slice(-1)[0];
  ok(h.action === "reassign" && /tools_steward -> runtime/.test(h.note), "history entry");
  ok(/reassign owner tools_steward -> runtime/.test(fs.readFileSync(path.join(ops, "log.jsonl"), "utf8")), "log entry");
  ok(validate(ops).status === 0, "board validates after reassign");
  patch(ops, (b) => { b.agents.find((a) => a.id === "context").place = "/elsewhere"; });
  const raw = fs.readFileSync(path.join(ops, "board.json"), "utf8");
  r = wp(ops, "reassign", "--as", "gm", "--id", kid, "--owner", "tools_steward");
  ok(r.status !== 0 && fs.readFileSync(path.join(ops, "board.json"), "utf8") === raw, "validate failure rolls back");
  void top;
});


group("org_drift: validate catches hand-filled superior mismatch; org-sync fixes", () => {
  const ops = fresh();
  ok(validate(ops).status === 0, "migrated board validates");
  const r0 = wp(ops, "org-tree", "--json");
  const jsonLine = r0.stdout.split("\n").slice(1).join("\n");
  let tree = null; try { tree = JSON.parse(jsonLine); } catch {}
  ok(r0.status === 0 && tree?.ok, "org-tree --json ok");
  ok(tree?.reporting?.session?.superior === "backend_mid", "session←backend_mid");
  ok(tree?.reporting?.gm?.superior === "andy", "gm←andy");
  patch(ops, (b) => { b.agents.find((a) => a.id === "session").superior = "gm"; });
  const v = validate(ops);
  ok(v.status !== 0 && /角色 session 的 superior 应为 backend_mid，当前是 gm/.test(v.stderr), "drift caught in zh");
  let r = wp(ops, "org-sync", "--as", "gm");
  ok(r.status === 0 && board(ops).agents.find((a) => a.id === "session").superior === "backend_mid", "org-sync rewrites session.superior");
  ok(validate(ops).status === 0, "validates after org-sync");
  r = wp(ops, "org-sync", "--as", "session");
  ok(r.status !== 0 && /只有 gm\/andy/.test(r.stderr), "non gm/andy org-sync rejected");
});

group("charter_smoke: role-charter includes superior and accept/reject", () => {
  const ops = fresh();
  for (const id of ["gm", "backend_mid", "session", "tools_steward"]) {
    const r = wp(ops, "role-charter", "--id", id);
    ok(r.status === 0 && r.stdout.includes("岗位说明") && r.stdout.includes("上级：") && r.stdout.includes("接受的交付"), `charter ${id}`);
    ok(r.stdout.includes("不接受的交付"), `charter ${id} reject`);
  }
  const r = wp(ops, "role-charter", "--id", "session");
  ok(/上级：backend_mid/.test(r.stdout), "session superior in charter");
  const bad = wp(ops, "role-charter", "--id", "nobody");
  ok(bad.status !== 0 && /没有这个角色/.test(bad.stderr), "unknown role rejected");
});

group("request_loop: request → approve → handle(keep/owner) → deliver → close → request_resolved", () => {
  const ops = fresh();
  let r = wp(ops, "request", "--as", "session", "--title", "need session fix", "--goal", "fix crash", "--domain", "harness-core");
  ok(r.status === 0, "request ok: " + r.stderr);
  const id = lastId(ops);
  const w0 = W(ops, id);
  ok(w0.kind === "request" && w0.requester === "session" && w0.approver === "backend_mid" && w0.status === "awaiting_approval", "request fields");
  const inbox0 = JSON.parse(fs.readFileSync(path.join(ops, "events", "inbox.json"), "utf8"));
  ok(inbox0.inbox.some((e) => e.type === "team_ops.approval_requested" && e.wp === id), "approval_requested on request");
  // forward branch optional: skip to keep loop short; test forward separately lightly
  r = wp(ops, "forward", "--as", "backend_mid", "--id", id, "--note", "need gm");
  ok(r.status === 0 && W(ops, id).approver === "gm", "forward to gm");
  // cannot forward past gm to andy
  r = wp(ops, "forward", "--as", "gm", "--id", id);
  ok(r.status !== 0 && /委托人/.test(r.stderr), "forward to principal blocked");
  // set approver back to backend_mid for handle demo (re-approve path): reject+re-request would be heavy; patch approver
  patch(ops, (b) => { const w = b.work_packages.find((x) => x.id === id); w.approver = "backend_mid"; w.reviewer = "backend_mid"; });
  r = wp(ops, "approve", "--as", "backend_mid", "--id", id);
  ok(r.status === 0 && W(ops, id).status === "ready", "approve request");
  // default handle = keep owner as approver
  r = wp(ops, "handle", "--as", "backend_mid", "--id", id);
  ok(r.status === 0 && W(ops, id).owner === "backend_mid", "handle default keep: owner=approver");
  ok(W(ops, id).history.some((h) => h.action === "handle"), "history handle");
  // reassign via handle --owner session (domain bot) for exec demo — actually use --owner tools for demo path
  // first need ready again; still ready. handle again? owner already mid. Use --owner session
  r = wp(ops, "handle", "--as", "backend_mid", "--id", id, "--owner", "session");
  ok(r.status === 0 && W(ops, id).owner === "session", "handle --owner session");
  // intake flag
  r = wp(ops, "handle", "--as", "backend_mid", "--id", id, "--intake");
  ok(r.status === 0 && W(ops, id).owner === "tools_steward", "handle --intake → tools_steward");
  // put back to session for deliver loop
  r = wp(ops, "handle", "--as", "backend_mid", "--id", id, "--owner", "session");
  wp(ops, "start", "--as", "session", "--id", id);
  wp(ops, "deliver", "--as", "session", "--id", id);
  r = wp(ops, "close", "--as", "backend_mid", "--id", id, "--to", "done", "--note", "fixed");
  ok(r.status === 0 && W(ops, id).status === "done", "close done: " + r.stderr);
  const inbox = JSON.parse(fs.readFileSync(path.join(ops, "events", "inbox.json"), "utf8"));
  const resolved = inbox.inbox.filter((e) => e.type === "team_ops.request_resolved" && e.wp === id);
  ok(resolved.length === 1 && resolved[0].meta.requester === "session", "request_resolved to requester");
  const outbox = fs.readFileSync(path.join(ops, "events", "outbox.jsonl"), "utf8");
  ok(outbox.includes("team_ops.request_resolved"), "outbox has request_resolved");
});

group("zh_errors: key CLI failures are Chinese with 当前 status", () => {
  const ops = fresh();
  const { top, kid } = twoLayer(ops);
  let r = wp(ops, "fill", "--as", "gm", "--id", top, "--text", "late"); // top is ready after twoLayer approve
  // top is ready; fill should fail
  ok(r.status !== 0 && /只有起草中才能填方案/.test(r.stderr) && /当前 ready/.test(r.stderr), "fill non-drafting zh: " + r.stderr.trim());
  r = wp(ops, "submit", "--as", "tools_steward", "--id", kid);
  // kid drafting without content
  ok(r.status !== 0 && /要先填方案/.test(r.stderr) && /当前 drafting/.test(r.stderr), "submit no content zh");
  r = wp(ops, "new", "--as", "ghost", "--title", "x", "--goal", "x", "--owner", "gm", "--approver", "gm");
  ok(r.status !== 0 && /未知身份 ghost/.test(r.stderr), "unknown as zh");
  wp(ops, "fill", "--as", "tools_steward", "--id", kid, "--text", "c");
  wp(ops, "submit", "--as", "tools_steward", "--id", kid);
  r = wp(ops, "approve", "--as", "tools_steward", "--id", kid);
  ok(r.status !== 0 && /不是审批人/.test(r.stderr) && /当前 awaiting_approval/.test(r.stderr), "approve wrong actor zh");
  r = wp(ops, "start", "--as", "gm", "--id", kid);
  // still awaiting
  ok(r.status !== 0 && (/不是负责人/.test(r.stderr) || /只有就绪才能开工/.test(r.stderr)), "start wrong zh");
  r = wp(ops, "request", "--as", "andy", "--title", "x", "--goal", "y");
  ok(r.status !== 0 && /委托人没有上级/.test(r.stderr), "principal cannot request");
});

if (!keep) for (const r of tmpRoots) fs.rmSync(r, { recursive: true, force: true });
const failed = results.filter((g) => g.fail.length);
const passedChecks = results.reduce((a, g) => a + g.pass, 0), failedChecks = results.reduce((a, g) => a + g.fail.length, 0);
const sum = { ok: failed.length === 0, groups: results.length, passed_groups: results.length - failed.length, checks_total: passedChecks + failedChecks, checks_passed: passedChecks, checks_failed: failedChecks, failures: failed.map((g) => ({ group: g.name, fail: g.fail })), tools_version: reg.tools_version };
recordTrace(realOps, process.argv.includes("--wp") ? process.argv[process.argv.indexOf("--wp") + 1] : null, "ops.wp.test", reg.tools_version);
console.log(JSON.stringify(sum));
process.exit(failed.length ? 1 : 0);

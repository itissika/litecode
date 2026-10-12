// ops.wp (v3, tools_version 15; v3 adds `set`, durable events) — nested WP lifecycle. Plan approval and completion acceptance are separate:
//   drafting --submit--> awaiting_approval --approve--> ready --start--> in_progress --deliver--> review --close--> done|blocked
//   reject on awaiting_approval → drafting (plan);  reject on review → in_progress (acceptance, optional --reopen children)
//   A child cannot submit its plan or start until its parent's plan is approved.
// Chain of command for TOOL REQUESTS: domain bots → their middle manager, who forwards approved ones directly to
//   tools_steward; middle managers' own requests → GM; GM's own requests → ask Andy first; tools_steward only
//   accepts requests forwarded by its superior.
// Identity: --as <agent_id|andy> is SELF-DECLARED (like wp_gate --source). No real auth — known limitation.
// Every write: board.json written → validate.mjs → rollback on failure; history + log.jsonl appended;
// board.md「WP 状态」re-rendered (tree); tool_traces recorded.
import fs from "node:fs";
import path from "node:path";
import {
  loadRegistry, printVersionHeader, opsRoot, recordTrace, renderWpStatusTable, applyStatusEnteredAt,
  wpById, childrenOf, approverOf, isApproved, isNewModelWp, wpTreeOrder, wpChain, CLOSED_STATUSES, writeBoardValidated, appendOpsLog, mutateInbox, newEventId, escalationTarget,
} from "./_lib.mjs";

const TOOL = "ops.wp";
const { reg } = loadRegistry(import.meta.url);
printVersionHeader(reg, TOOL);
const ops = opsRoot(import.meta.url);
const boardPath = path.join(ops, "board.json");
const boardRaw = fs.readFileSync(boardPath, "utf8");
const board = JSON.parse(boardRaw);

const argv = process.argv.slice(2);
const cmd = argv[0];
const rest = argv.slice(1);
const opt = (n) => { const i = rest.indexOf(n); return i >= 0 ? rest[i + 1] ?? null : null; };
const flag = (n) => rest.includes(n);
const USAGE = `usage: wp.mjs <cmd> --as <agent_id|andy> ...   (v2: plan approval and completion acceptance are separate)
  new      --title T --goal G --owner O [--parent WP-x] [--approver A] [--domain D] [--result R] [--done-when "a;b"] [--artifact P]
  fill     --id WP-x (--text T | --content-file F) [--kind direct|split]   # owner, drafting only
  submit   --id WP-x [--note N]          # owner: plan → awaiting_approval (split needs ≥1 child; children may be drafting; parent plan must be approved)
  approve  --id WP-x [--note N]          # approver: plan approved → ready (may execute and delegate down)
  start    --id WP-x [--note N]          # owner: ready → in_progress (plan + parent plan approved)
  deliver  --id WP-x [--note N]          # owner: submit for acceptance → review (all children done/blocked/cancelled)
  reject   --id WP-x --note N [--reopen WP-a,WP-b]   # approver: plan → drafting | acceptance → in_progress (+reopen children)
  close    --id WP-x --to done|blocked|cancelled [--note N]   # approver: done only from review
  set      --id WP-x --field artifact_path|handoff_path|progress|result|done_when --value V   # owner or approver; not when closed
  tree     [--id WP-x] [--json]
  summary  --id WP-x [--json]
  history  --id WP-x [--json]`;

function die(msg, code = 1) { console.error(msg); process.exit(code); }
const known = new Set([...board.agents.map((a) => a.id), "andy"]);
const as = opt("--as");
const READ = new Set(["tree", "summary", "history"]);
if (!cmd || !["new", "fill", "submit", "approve", "start", "deliver", "reject", "close", "set", ...READ].includes(cmd)) die(USAGE, 2);
if (!READ.has(cmd)) {
  if (!as) die("--as <agent_id|andy> required (self-declared identity)", 2);
  if (!known.has(as)) die(`--as ${as}: unknown agent`, 2);
}
const nowIso = new Date().toISOString();
function need(id) { const w = wpById(board, id); if (!w) die(`missing WP ${id}`); return w; }
function hist(w, action, note = null, extra = {}) {
  if (!Array.isArray(w.history)) w.history = [];
  w.history.push({ ts: nowIso, actor: as, action, note: note ?? null, ...extra });
}
function setStatus(w, to) { w.status = to; applyStatusEnteredAt(w, to, nowIso); }
function requireActor(who, role, id) { if (as !== who) die(`${cmd} ${id}: --as ${as} is not ${role} (${who})`); }
function emit(type, summary, wp, meta) { // v15: locked + durable; deliverable → outbox before send
  const ev = { id: newEventId(wp.replace(/\W/g, "") + "-" + type.split(".").pop()), ts: new Date().toISOString(), type, source: TOOL, summary, wp, acked: [], meta: { ...meta, target: escalationTarget(meta.approver) } }; // target never andy: GM decides whether to tell Andy
  mutateInbox(ops, (box) => { box.inbox.push(ev); });
  return ev.id;
}
function commit(id, summary, extraOut = {}) {
  const r = writeBoardValidated(ops, board, boardRaw);
  if (!r.ok) { process.stderr.write(r.stderr); die("validate failed after write; board.json restored, nothing changed"); }
  appendOpsLog(ops, { kind: "wp_lifecycle", source: as, summary: `${summary} (ops.wp ${cmd})`, evidence: [".litecode/ops/board.json"] });
  const t = renderWpStatusTable(ops);
  recordTrace(ops, id, TOOL, reg.tools_version);
  console.log(JSON.stringify({ ok: true, cmd, id, as, written: true, validated: true, board_md_wp_table: t.rows, tools_version: reg.tools_version, ...extraOut }));
}
function nextId() {
  const n = Math.max(0, ...board.work_packages.map((w) => Number(w.id.slice(3)) || 0)) + 1;
  return "WP-" + String(n).padStart(3, "0");
}
/** Rule: children cannot submit plans or start until the parent's plan is approved. Old-model parents count as approved. */
function parentPlanApproved(w) {
  if (!w.parent) return true;
  const p = wpById(board, w.parent); if (!p) return true;
  return isNewModelWp(p) ? isApproved(p) : true;
}
function openKids(id) { return childrenOf(board, id).filter((c) => !CLOSED_STATUSES.has(c.status)); }

switch (cmd) {
  case "new": {
    const title = opt("--title"), goal = opt("--goal"), owner = opt("--owner"), parentId = opt("--parent");
    if (!title || !goal || !owner) die("new: --title --goal --owner required", 2);
    let parent = null, approver = opt("--approver");
    if (parentId) {
      parent = need(parentId);
      if (CLOSED_STATUSES.has(parent.status)) die(`new: parent ${parentId} is ${parent.status}`);
      requireActor(parent.owner, "parent owner", parentId); // upper level cannot create on behalf of lower
      approver = approver ?? parent.owner;
    } else {
      if (as !== "gm" && as !== "andy") die("new: top-level packages only by gm/andy");
      if (!approver) die("new: top-level package needs --approver", 2);
    }
    const id = nextId();
    const w = {
      id, domain: opt("--domain") ?? parent?.domain ?? "team-ops", title,
      result: opt("--result") ?? goal,
      done_when: (opt("--done-when") ?? "approver close done").split(";").map((x) => x.trim()).filter(Boolean),
      owner, reviewer: approver, status: "drafting", depends_on: [], blocker: null, handoff_path: null,
      artifact_path: opt("--artifact") ?? null, progress: "drafting", status_entered_at: null,
      parent: parentId ?? null, goal, approver, content: null, history: [],
    };
    hist(w, "create", `goal: ${goal}`);
    board.work_packages.push(w);
    commit(id, `${id}: created (parent=${parentId ?? "-"} owner=${owner} approver=${approver})`, { parent: parentId ?? null, approver, status: "drafting" });
    break;
  }
  case "fill": {
    const id = opt("--id"); const w = need(id);
    requireActor(w.owner, "owner", id);
    if (w.status !== "drafting") die(`fill ${id}: only in drafting (is ${w.status})`);
    let text = opt("--text"); const f = opt("--content-file");
    if (f) text = fs.readFileSync(f, "utf8");
    if (!text) die("fill: --text or --content-file required", 2);
    const kind = opt("--kind") ?? "direct";
    if (!["direct", "split"].includes(kind)) die("fill: --kind direct|split", 2);
    w.content = { kind, text };
    hist(w, "fill", `${kind}: ${text.slice(0, 80)}`);
    commit(id, `${id}: content filled (${kind})`);
    break;
  }
  case "submit": { // plan submission
    const id = opt("--id"); const w = need(id);
    requireActor(w.owner, "owner", id);
    if (w.status !== "drafting") die(`submit ${id}: only from drafting (is ${w.status})`);
    if (!w.content || !w.content.text) die(`submit ${id}: fill content first`);
    if (!parentPlanApproved(w)) die(`submit ${id}: parent ${w.parent} plan not approved yet`);
    if (w.content.kind === "split" && childrenOf(board, id).length === 0) die(`submit ${id}: kind=split needs at least one child`);
    setStatus(w, "awaiting_approval"); w.progress = `plan awaiting approval by ${approverOf(w)}`;
    hist(w, "submit", opt("--note"));
    const ev = emit("team_ops.approval_requested", `${id} plan awaiting approval by ${approverOf(w)} (owner ${w.owner})`, id, { stage: "plan", approver: approverOf(w), owner: w.owner, path: wpChain(board, id) });
    commit(id, `${id}: drafting -> awaiting_approval (plan; approver ${approverOf(w)})`, { event: ev });
    break;
  }
  case "approve": { // plan approval: allows execution and delegating down
    const id = opt("--id"); const w = need(id);
    requireActor(approverOf(w), "approver", id);
    if (w.status !== "awaiting_approval") die(`approve ${id}: only from awaiting_approval (is ${w.status})`);
    setStatus(w, "ready"); w.progress = `plan approved by ${as}`;
    hist(w, "approve", opt("--note"));
    commit(id, `${id}: awaiting_approval -> ready (plan approved by ${as})`);
    break;
  }
  case "start": {
    const id = opt("--id"); const w = need(id);
    requireActor(w.owner, "owner", id);
    if (w.status !== "ready") die(`start ${id}: only from ready (is ${w.status})`);
    if (!isApproved(w)) die(`start ${id}: plan not approved`);
    if (!parentPlanApproved(w)) die(`start ${id}: parent ${w.parent} plan not approved yet`);
    setStatus(w, "in_progress"); w.progress = "in progress";
    hist(w, "start", opt("--note"));
    commit(id, `${id}: ready -> in_progress`);
    break;
  }
  case "deliver": { // completion: submit for acceptance
    const id = opt("--id"); const w = need(id);
    requireActor(w.owner, "owner", id);
    if (!["ready", "in_progress"].includes(w.status)) die(`deliver ${id}: only from ready/in_progress (is ${w.status})`);
    if (!isApproved(w)) die(`deliver ${id}: plan not approved`);
    const ok = openKids(id);
    if (ok.length) die(`deliver ${id}: children not closed (need done/blocked): ${ok.map((c) => c.id + "(" + c.status + ")").join(", ")}`);
    const from = w.status;
    setStatus(w, "review"); w.progress = `awaiting acceptance by ${approverOf(w)}`;
    hist(w, "deliver", opt("--note"));
    const ev = emit("team_ops.acceptance_requested", `${id} awaiting acceptance by ${approverOf(w)} (owner ${w.owner})`, id, { stage: "acceptance", approver: approverOf(w), owner: w.owner, path: wpChain(board, id) });
    commit(id, `${id}: ${from} -> review (submitted for acceptance; approver ${approverOf(w)})`, { event: ev });
    break;
  }
  case "reject": {
    const id = opt("--id"); const w = need(id); const note = opt("--note");
    requireActor(approverOf(w), "approver", id);
    if (!note) die("reject: --note required", 2);
    if (!["awaiting_approval", "review"].includes(w.status)) die(`reject ${id}: only from awaiting_approval/review (is ${w.status})`);
    const reopen = (opt("--reopen") ?? "").split(",").map((x) => x.trim()).filter(Boolean);
    if (reopen.length && w.status !== "review") die(`reject ${id}: --reopen only on acceptance rejection (review)`);
    for (const cid of reopen) {
      const c = need(cid);
      if (c.parent !== id) die(`reject ${id}: --reopen ${cid} is not a child of ${id}`);
      if (!["done", "blocked"].includes(c.status)) die(`reject ${id}: --reopen ${cid} is ${c.status} (needs done/blocked)`);
      setStatus(c, "drafting"); c.progress = `reopened by ${as} via ${id}: ${note}`;
      hist(c, "reopen", `via ${id}: ${note}`);
    }
    const from = w.status;
    if (from === "awaiting_approval") { // plan rejection → drafting
      setStatus(w, "drafting"); w.progress = `plan rejected by ${as}: ${note}`;
      hist(w, "reject", note);
    } else { // acceptance rejection → back to in_progress; plan stays approved
      setStatus(w, "in_progress"); w.progress = `acceptance rejected by ${as}: ${note}`;
      hist(w, "return", note);
    }
    commit(id, `${id}: ${from} -> ${w.status} (${from === "review" ? "acceptance" : "plan"} rejected by ${as}${reopen.length ? "; reopened " + reopen.join(",") : ""})`, { reopened: reopen, stage: from === "review" ? "acceptance" : "plan" });
    break;
  }
  case "close": {
    const id = opt("--id"); const w = need(id); const to = opt("--to"); const note = opt("--note");
    requireActor(approverOf(w), "approver", id);
    if (!["done", "blocked", "cancelled"].includes(to)) die("close: --to done|blocked|cancelled", 2);
    if (to === "done" && w.status !== "review") die(`close ${id}: done only from review (deliver first; is ${w.status})`);
    if (to === "done" && !isApproved(w)) die(`close ${id}: plan not approved`);
    if (to !== "cancelled") {
      const ok = openKids(id);
      if (ok.length) die(`close ${id}: children not closed: ${ok.map((c) => c.id + "(" + c.status + ")").join(", ")}`);
    }
    const blockedKids = childrenOf(board, id).filter((c) => c.status === "blocked");
    if ((blockedKids.length || to === "blocked") && !note) die(`close ${id}: --note required (blocked ${to === "blocked" ? "self" : "children: " + blockedKids.map((c) => c.id).join(",")})`, 2);
    const from = w.status;
    setStatus(w, to); w.progress = `closed ${to} by ${as}${note ? ": " + note : ""}`;
    if (to === "blocked") w.blocker = w.blocker ?? { kind: "architecture", note };
    hist(w, "close", note, { to });
    commit(id, `${id}: ${from} -> ${to} (closed by ${as})`);
    break;
  }
  case "set": { // v3: owner/approver edit safe descriptive fields; validate + rollback like other writes
    const id = opt("--id"); const w = need(id); const field = opt("--field"); const value = opt("--value");
    const SAFE = ["artifact_path", "handoff_path", "progress", "result", "done_when"];
    if (!field || value === null) die("set: --field F --value V required", 2);
    if (!SAFE.includes(field)) die(`set: field ${field} not allowed (safe: ${SAFE.join(", ")})`, 2);
    if (as !== w.owner && as !== approverOf(w)) die(`set ${id}: --as ${as} is neither owner (${w.owner}) nor approver (${approverOf(w)})`);
    if (CLOSED_STATUSES.has(w.status)) die(`set ${id}: package is ${w.status}`);
    let v = value === "" || value === "null" ? null : value;
    if (field === "artifact_path" || field === "handoff_path") {
      if (v !== null && !/^\.litecode\/ops\/[^\s]+$/.test(v)) die(`set: ${field} must be a repo-relative path under .litecode/ops/ (convention), got ${v}`, 2);
    }
    if (field === "done_when") { v = String(value).split(";").map((x) => x.trim()).filter(Boolean); if (!v.length) die("set: done_when needs ≥1 item", 2); }
    if ((field === "result" || field === "progress") && v === null) die(`set: ${field} cannot be null`, 2);
    const old = w[field];
    w[field] = v;
    hist(w, "set", `${field}: ${JSON.stringify(old ?? null).slice(0, 60)} -> ${JSON.stringify(v).slice(0, 60)}`);
    commit(id, `${id}: set ${field}`, { field, value: v });
    break;
  }
  case "tree": {
    const id = opt("--id");
    if (id) need(id);
    const rows = wpTreeOrder(board, id).map(({ w, depth }) => ({ depth, id: w.id, status: w.status, owner: w.owner, approver: approverOf(w), title: w.title, artifact: w.artifact_path ?? null }));
    if (flag("--json")) console.log(JSON.stringify({ ok: true, root: id ?? null, rows }, null, 2));
    else for (const r of rows) console.log(`${"  ".repeat(r.depth)}${r.depth ? "└ " : ""}${r.id} [${r.status}] owner=${r.owner} approver=${r.approver} ${r.title}${r.artifact ? "  → " + r.artifact : ""}`);
    recordTrace(ops, id, TOOL, reg.tools_version);
    break;
  }
  case "summary": {
    const id = opt("--id"); if (!id) die("summary: --id required", 2); const w = need(id);
    const kids = wpTreeOrder(board, id).slice(1).map(({ w: c, depth }) => ({ depth, id: c.id, status: c.status, owner: c.owner, approver: approverOf(c), artifact: c.artifact_path ?? null, last: (c.history || []).slice(-1)[0] ?? null }));
    const out = { ok: true, id, status: w.status, approver: approverOf(w), approved: isApproved(w), path: wpChain(board, id), content_kind: w.content?.kind ?? null,
      children: kids, open_children: kids.filter((k) => !CLOSED_STATUSES.has(k.status)).map((k) => k.id), last: (w.history || []).slice(-1)[0] ?? null };
    if (flag("--json")) console.log(JSON.stringify(out, null, 2));
    else {
      console.log(`${id} [${w.status}] approver=${out.approver} approved=${out.approved} path=${out.path.join(" > ")} kind=${out.content_kind ?? "-"}`);
      for (const k of kids) console.log(`${"  ".repeat(k.depth)}- ${k.id} [${k.status}] owner=${k.owner} artifact=${k.artifact ?? "-"} last=${k.last ? k.last.action + "@" + k.last.actor : "-"}`);
      console.log(`open children: ${out.open_children.join(",") || "none"}`);
    }
    recordTrace(ops, id, TOOL, reg.tools_version);
    break;
  }
  case "history": {
    const id = opt("--id"); if (!id) die("history: --id required", 2); const w = need(id);
    const h = w.history || [];
    if (flag("--json")) console.log(JSON.stringify({ ok: true, id, history: h }, null, 2));
    else for (const e of h) console.log(`${e.ts} ${e.actor} ${e.action}${e.to ? "→" + e.to : ""}${e.note ? "  " + e.note : ""}`);
    recordTrace(ops, id, TOOL, reg.tools_version);
    break;
  }
}

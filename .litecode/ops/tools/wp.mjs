// ops.wp (v7, tools_version 24; WP-189: org-tree/org-sync/role-charter/request/handle/forward + zh errors)
//   drafting --submit--> awaiting_approval --approve--> ready --start--> in_progress --deliver--> review --close--> done|blocked
//   reject on awaiting_approval → drafting (plan);  reject on review → in_progress (acceptance, optional --reopen children)
//   A child cannot submit its plan or start until its parent's plan is approved.
//   WP-189: request → approve → handle → start/deliver → close(done) emits team_ops.request_resolved to requester.
// Identity: --as <agent_id|andy> is SELF-DECLARED (like wp_gate --source). No real auth — known limitation.
import fs from "node:fs";
import path from "node:path";
import {
  loadRegistry, printVersionHeader, opsRoot, recordTrace, renderWpStatusTable, applyStatusEnteredAt,
  wpById, childrenOf, softEnsureWatcher, approverOf, isApproved, isNewModelWp, wpTreeOrder, wpChain, CLOSED_STATUSES,
  writeBoardValidated, appendOpsLog, mutateInbox, newEventId, escalationTarget,
  deriveReporting, applyDerivedReporting, orgTreeRows, opsConfigOf, superiorOfAgent, charterOf,
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
const USAGE = `usage: wp.mjs <cmd> --as <agent_id|andy> ...   (v2: 方案审批与完工验收分开；v7 WP-189 组织树/向上请求)
  new      --title T --goal G --owner O [--parent WP-x] [--approver A] [--domain D] [--result R] [--done-when "a;b"] [--artifact P]
  fill     --id WP-x (--text T | --content-file F) [--kind direct|split]   # owner, drafting only
  submit   --id WP-x [--note N]          # owner: plan → awaiting_approval
  approve  --id WP-x [--note N]          # approver: plan approved → ready
  start    --id WP-x [--note N]          # owner: ready → in_progress
  deliver  --id WP-x [--note N]          # owner: → review
  reject   --id WP-x --note N [--reopen WP-a,WP-b]
  close    --id WP-x --to done|blocked|cancelled [--note N]
  set      --id WP-x --field artifact_path|handoff_path|progress|result|done_when|domain --value V
  reassign --id WP-x --owner O [--note N]
  request  --title T --goal G [--domain D] [--text T|--content-file F]   # 向上请求；approver=推导上级
  handle   --id WP-x [--owner AGENT|--intake]   # 审批人：默认自留 owner；--owner 指定；--intake 才用 intake_agent（仅 ops/工具类）
  forward  --id WP-x [--note N]          # 审批人：上交自己的上级
  org-tree [--json]                      # 只读：组织树
  org-sync                               # gm/andy：按树重写 superior/subordinates
  role-charter --id AGENT                # 只读：岗位提示词 markdown
  tree     [--id WP-x] [--json]
  summary  --id WP-x [--json]
  history  --id WP-x [--json]`;

function die(msg, code = 1) { console.error(msg); process.exit(code); }
/** 关键 CLI 错误：直白中文 + 当前状态（成功路径 JSON 字段名保持英文） */
function dieZh(action, id, reason, status = null, code = 1) {
  const cur = status != null ? `（当前 ${status}）` : "";
  const head = id ? `${action} ${id}` : action;
  die(`${head}：${reason}${cur}`, code);
}
const known = new Set([...board.agents.map((a) => a.id), "andy"]);
const as = opt("--as");
const READ = new Set(["tree", "summary", "history", "org-tree", "role-charter"]);
const CMDS = ["new", "fill", "submit", "approve", "start", "deliver", "reject", "close", "set", "reassign", "request", "handle", "forward", "org-sync", ...READ];
if (!cmd || !CMDS.includes(cmd)) die(USAGE, 2);
if (!READ.has(cmd)) {
  if (!as) dieZh(cmd, null, "需要 --as <agent_id|andy>（身份自报）", null, 2);
  if (!known.has(as)) dieZh(cmd, null, `未知身份 ${as}`, null, 2);
}
const nowIso = new Date().toISOString();
function need(id) { const w = wpById(board, id); if (!w) dieZh(cmd, id, "找不到工作包"); return w; }
function hist(w, action, note = null, extra = {}) {
  if (!Array.isArray(w.history)) w.history = [];
  w.history.push({ ts: nowIso, actor: as, action, note: note ?? null, ...extra });
}
function setStatus(w, to) { w.status = to; applyStatusEnteredAt(w, to, nowIso); }
function requireActor(who, roleZh, id) {
  if (as !== who) dieZh(cmd, id, `--as ${as} 不是${roleZh}（${who}）`, wpById(board, id)?.status);
}
function emit(type, summary, wp, meta) {
  const tgt = meta.target ?? meta.approver;
  const ev = {
    id: newEventId(wp.replace(/\W/g, "") + "-" + type.split(".").pop()),
    ts: new Date().toISOString(), type, source: TOOL, summary, wp, acked: [],
    meta: { ...meta, target: escalationTarget(tgt) },
  };
  mutateInbox(ops, (box) => { box.inbox.push(ev); });
  return ev.id;
}
function commit(id, summary, extraOut = {}) {
  const r = writeBoardValidated(ops, board, boardRaw);
  if (!r.ok) { process.stderr.write(r.stderr); dieZh(cmd, id, "校验失败，board.json 已恢复，未改动"); }
  appendOpsLog(ops, { kind: "wp_lifecycle", source: as, summary: `${summary} (ops.wp ${cmd})`, evidence: [".litecode/ops/board.json"] });
  const t = renderWpStatusTable(ops);
  recordTrace(ops, id, TOOL, reg.tools_version);
  softEnsureWatcher(ops);
  console.log(JSON.stringify({ ok: true, cmd, id, as, written: true, validated: true, board_md_wp_table: t.rows, tools_version: reg.tools_version, ...extraOut }));
}
function nextId() {
  const n = Math.max(0, ...board.work_packages.map((w) => Number(w.id.slice(3)) || 0)) + 1;
  return "WP-" + String(n).padStart(3, "0");
}
function parentPlanApproved(w) {
  if (!w.parent) return true;
  const p = wpById(board, w.parent); if (!p) return true;
  return isNewModelWp(p) ? isApproved(p) : true;
}
function openKids(id) { return childrenOf(board, id).filter((c) => !CLOSED_STATUSES.has(c.status)); }
function agentIds() { return board.agents.map((a) => a.id); }

switch (cmd) {
  case "new": {
    const title = opt("--title"), goal = opt("--goal"), owner = opt("--owner"), parentId = opt("--parent");
    if (!title || !goal || !owner) dieZh("new", null, "需要 --title --goal --owner", null, 2);
    let parent = null, approver = opt("--approver");
    if (parentId) {
      parent = need(parentId);
      if (CLOSED_STATUSES.has(parent.status)) dieZh("new", parentId, `父包已是 ${parent.status}`, parent.status);
      requireActor(parent.owner, "父包负责人", parentId);
      approver = approver ?? parent.owner;
    } else {
      if (as !== "gm" && as !== "andy") dieZh("new", null, "顶层包只能由 gm/andy 创建");
      if (!approver) dieZh("new", null, "顶层包需要 --approver", null, 2);
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
    requireActor(w.owner, "负责人", id);
    if (w.status !== "drafting") dieZh("fill", id, "只有起草中才能填方案", w.status);
    let text = opt("--text"); const f = opt("--content-file");
    if (f) text = fs.readFileSync(f, "utf8");
    if (!text) dieZh("fill", id, "需要 --text 或 --content-file", w.status, 2);
    const kind = opt("--kind") ?? "direct";
    if (!["direct", "split"].includes(kind)) dieZh("fill", id, "kind 只能是 direct 或 split", w.status, 2);
    w.content = { kind, text };
    hist(w, "fill", `${kind}: ${text.slice(0, 80)}`);
    commit(id, `${id}: content filled (${kind})`);
    break;
  }
  case "submit": {
    const id = opt("--id"); const w = need(id);
    requireActor(w.owner, "负责人", id);
    if (w.status !== "drafting") dieZh("submit", id, "只有起草中才能提交", w.status);
    if (!w.content || !w.content.text) dieZh("submit", id, "要先填方案", w.status);
    if (!parentPlanApproved(w)) dieZh("submit", id, `父包 ${w.parent} 方案还没批`, w.status);
    if (w.content.kind === "split" && childrenOf(board, id).length === 0) dieZh("submit", id, "是拆分包，至少要有一个子包", w.status);
    setStatus(w, "awaiting_approval"); w.progress = `plan awaiting approval by ${approverOf(w)}`;
    hist(w, "submit", opt("--note"));
    const ev = emit("team_ops.approval_requested", `${id} plan awaiting approval by ${approverOf(w)} (owner ${w.owner})`, id, { stage: "plan", approver: approverOf(w), owner: w.owner, path: wpChain(board, id) });
    commit(id, `${id}: drafting -> awaiting_approval (plan; approver ${approverOf(w)})`, { event: ev });
    break;
  }
  case "approve": {
    const id = opt("--id"); const w = need(id);
    requireActor(approverOf(w), "审批人", id);
    if (w.status !== "awaiting_approval") dieZh("approve", id, "只有待审批才能批准", w.status);
    setStatus(w, "ready"); w.progress = `plan approved by ${as}`;
    hist(w, "approve", opt("--note"));
    commit(id, `${id}: awaiting_approval -> ready (plan approved by ${as})`);
    break;
  }
  case "start": {
    const id = opt("--id"); const w = need(id);
    requireActor(w.owner, "负责人", id);
    if (w.status !== "ready") dieZh("start", id, "只有就绪才能开工", w.status);
    if (!isApproved(w)) dieZh("start", id, "方案还没批", w.status);
    if (!parentPlanApproved(w)) dieZh("start", id, `父包 ${w.parent} 方案还没批`, w.status);
    setStatus(w, "in_progress"); w.progress = "in progress";
    hist(w, "start", opt("--note"));
    commit(id, `${id}: ready -> in_progress`);
    break;
  }
  case "deliver": {
    const id = opt("--id"); const w = need(id);
    requireActor(w.owner, "负责人", id);
    if (!["ready", "in_progress"].includes(w.status)) dieZh("deliver", id, "只有就绪或进行中才能送验收", w.status);
    if (!isApproved(w)) dieZh("deliver", id, "方案还没批", w.status);
    const ok = openKids(id);
    if (ok.length) dieZh("deliver", id, `还有未封子包：${ok.map((c) => c.id + "(" + c.status + ")").join(", ")}`, w.status);
    const from = w.status;
    setStatus(w, "review"); w.progress = `awaiting acceptance by ${approverOf(w)}`;
    hist(w, "deliver", opt("--note"));
    const ev = emit("team_ops.acceptance_requested", `${id} awaiting acceptance by ${approverOf(w)} (owner ${w.owner})`, id, { stage: "acceptance", approver: approverOf(w), owner: w.owner, path: wpChain(board, id) });
    commit(id, `${id}: ${from} -> review (submitted for acceptance; approver ${approverOf(w)})`, { event: ev });
    break;
  }
  case "reject": {
    const id = opt("--id"); const w = need(id); const note = opt("--note");
    requireActor(approverOf(w), "审批人", id);
    if (!note) dieZh("reject", id, "驳回必须写 --note", w.status, 2);
    if (!["awaiting_approval", "review"].includes(w.status)) dieZh("reject", id, "只有待审批或待验收才能驳回", w.status);
    const reopen = (opt("--reopen") ?? "").split(",").map((x) => x.trim()).filter(Boolean);
    if (reopen.length && w.status !== "review") dieZh("reject", id, "只有验收驳回才能 --reopen 子包", w.status);
    for (const cid of reopen) {
      const c = need(cid);
      if (c.parent !== id) dieZh("reject", id, `${cid} 不是 ${id} 的子包`, w.status);
      if (!["done", "blocked"].includes(c.status)) dieZh("reject", id, `${cid} 当前是 ${c.status}，只能重开 done 或 blocked`, c.status);
      setStatus(c, "drafting"); c.progress = `reopened by ${as} via ${id}: ${note}`;
      hist(c, "reopen", `via ${id}: ${note}`);
    }
    const from = w.status;
    if (from === "awaiting_approval") {
      setStatus(w, "drafting"); w.progress = `plan rejected by ${as}: ${note}`;
      hist(w, "reject", note);
    } else {
      setStatus(w, "in_progress"); w.progress = `acceptance rejected by ${as}: ${note}`;
      hist(w, "return", note);
    }
    commit(id, `${id}: ${from} -> ${w.status} (${from === "review" ? "acceptance" : "plan"} rejected by ${as}${reopen.length ? "; reopened " + reopen.join(",") : ""})`, { reopened: reopen, stage: from === "review" ? "acceptance" : "plan" });
    break;
  }
  case "close": {
    const id = opt("--id"); const w = need(id); const to = opt("--to"); const note = opt("--note");
    requireActor(approverOf(w), "审批人", id);
    if (!["done", "blocked", "cancelled"].includes(to)) dieZh("close", id, "--to 只能是 done、blocked 或 cancelled", w.status, 2);
    if (to === "done" && w.status !== "review") dieZh("close", id, "要先送验收，才能封为完成", w.status);
    if (to === "done" && !isApproved(w)) dieZh("close", id, "方案还没批", w.status);
    if (to !== "cancelled") {
      const ok = openKids(id);
      if (ok.length) dieZh("close", id, `还有未封子包：${ok.map((c) => c.id + "(" + c.status + ")").join(", ")}`, w.status);
    }
    const blockedKids = childrenOf(board, id).filter((c) => c.status === "blocked");
    const needNote = blockedKids.length || to === "blocked" || (w.kind === "request" && to !== "cancelled");
    if (needNote && !note) dieZh("close", id, "必须写 --note", w.status, 2);
    const from = w.status;
    setStatus(w, to); w.progress = `closed ${to} by ${as}${note ? ": " + note : ""}`;
    if (to === "blocked") w.blocker = w.blocker ?? { kind: "architecture", note };
    hist(w, "close", note, { to });
    let ev = null;
    if (w.kind === "request" && to === "done") {
      ev = emit("team_ops.request_resolved", `${id} 已完成：${note}（通知请求人 ${w.requester}）`, id, {
        target: w.requester, requester: w.requester, note, by: as,
      });
    }
    commit(id, `${id}: ${from} -> ${to} (closed by ${as})`, { event: ev });
    break;
  }
  case "set": {
    const id = opt("--id"); const w = need(id); const field = opt("--field"); const value = opt("--value");
    const SAFE = ["artifact_path", "handoff_path", "progress", "result", "done_when", "domain"];
    if (!field || value === null) dieZh("set", id, "需要 --field 和 --value", w?.status, 2);
    if (!SAFE.includes(field)) dieZh("set", id, `不能改 ${field}（允许：${SAFE.join(", ")}）`, w.status, 2);
    if (as !== w.owner && as !== approverOf(w)) dieZh("set", id, `--as ${as} 既不是负责人（${w.owner}）也不是审批人（${approverOf(w)}）`, w.status);
    if (CLOSED_STATUSES.has(w.status) && field !== "domain") dieZh("set", id, `已封包，只能改 domain`, w.status);
    let v = value === "" || value === "null" ? null : value;
    if (field === "artifact_path" || field === "handoff_path") {
      if (v !== null && !/^\.litecode\/ops\/[^\s]+$/.test(v)) dieZh("set", id, `${field} 必须是仓库内 .litecode/ops/ 下路径，收到 ${v}`, w.status, 2);
    }
    if (field === "done_when") { v = String(value).split(";").map((x) => x.trim()).filter(Boolean); if (!v.length) dieZh("set", id, "done_when 至少一项", w.status, 2); }
    if (field === "domain") {
      const ids = board.domains.map((d) => d.id);
      if (v === null || !ids.includes(v)) dieZh("set", id, `domain 必须是已有域 id（${ids.join(", ")}），收到 ${value}`, w.status, 2);
    }
    if ((field === "result" || field === "progress") && v === null) dieZh("set", id, `${field} 不能为空`, w.status, 2);
    const old = w[field];
    w[field] = v;
    hist(w, "set", `${field}: ${JSON.stringify(old ?? null).slice(0, 60)} -> ${JSON.stringify(v).slice(0, 60)}`);
    commit(id, `${id}: set ${field}`, { field, value: v });
    break;
  }
  case "reassign": {
    const id = opt("--id"); const w = need(id); const to = opt("--owner"); const note = opt("--note");
    if (!to) dieZh("reassign", id, "需要 --owner", w?.status, 2);
    if (!agentIds().includes(to)) dieZh("reassign", id, `--owner ${to} 不是已登记角色`, w.status, 2);
    const parent = w.parent ? wpById(board, w.parent) : null;
    const allowed = [approverOf(w), parent?.owner].filter(Boolean);
    if (!allowed.includes(as)) dieZh("reassign", id, `--as ${as} 既不是审批人（${approverOf(w)}）也不是父包负责人（${parent?.owner ?? "-"}）`, w.status);
    if (CLOSED_STATUSES.has(w.status)) dieZh("reassign", id, `已封包（${w.status}）`, w.status);
    if (w.owner === to) dieZh("reassign", id, `已经是 ${to}`, w.status, 2);
    const from = w.owner; w.owner = to;
    hist(w, "reassign", `owner: ${from} -> ${to}${note ? "; " + note : ""}`);
    commit(id, `${id}: reassign owner ${from} -> ${to}`, { from, to });
    break;
  }
  case "request": {
    const cfg = opsConfigOf(board);
    if (as === cfg.principal) dieZh("request", null, "委托人没有上级，请直接开工作包");
    const approver = superiorOfAgent(board, as);
    if (!approver) dieZh("request", null, `${as} 没有上级，不能向上请求`);
    const title = opt("--title"); const goal = opt("--goal");
    if (!title || !goal) dieZh("request", null, "需要 --title 和 --goal", null, 2);
    let text = opt("--text"); const f = opt("--content-file");
    if (f) text = fs.readFileSync(f, "utf8");
    text = text || goal;
    const domain = opt("--domain") ?? "team-ops";
    if (!board.domains.some((d) => d.id === domain)) dieZh("request", null, `域 ${domain} 不存在`, null, 2);
    const id = nextId();
    const w = {
      id, domain, title, result: goal,
      done_when: ["approver close done"],
      owner: as, reviewer: approver, status: "awaiting_approval", depends_on: [], blocker: null,
      handoff_path: null, artifact_path: null, progress: `等待 ${approver} 审批`,
      status_entered_at: nowIso, parent: null, goal, approver,
      kind: "request", requester: as,
      content: { kind: "direct", text },
      history: [],
    };
    hist(w, "create", `向上请求: ${goal}`);
    hist(w, "submit", "向上请求");
    board.work_packages.push(w);
    const ev = emit("team_ops.approval_requested", `${id} 向上请求，等待 ${approver} 审批（请求人 ${as}）`, id, {
      stage: "plan", approver, owner: as, requester: as, path: [id],
    });
    commit(id, `${id}: request → awaiting_approval (approver ${approver})`, { event: ev, approver, status: "awaiting_approval", kind: "request" });
    break;
  }
  case "handle": {
    // GM 口径覆盖原 plan §3.2：默认自留；--owner 指定；--intake 才落到 intake_agent（仅 ops/工具类）
    const id = opt("--id"); const w = need(id);
    if (w.kind !== "request") dieZh("handle", id, "不是请求包", w.status);
    requireActor(approverOf(w), "审批人", id);
    if (w.status !== "ready") dieZh("handle", id, "要先批准，才能转交/接手", w.status);
    const cfg = opsConfigOf(board);
    let owner = as; // 默认自留
    let mode = "keep";
    if (flag("--intake")) {
      const intake = cfg.intake_agent;
      if (!intake || !agentIds().includes(intake)) dieZh("handle", id, `intake_agent ${intake ?? "(空)"} 不存在或未配置`, w.status);
      const role = board.agents.find((a) => a.id === intake);
      if (role?.status === "fired") dieZh("handle", id, `intake_agent ${intake} 已解雇`, w.status);
      owner = intake;
      mode = "intake";
    } else if (opt("--owner")) {
      const o = opt("--owner");
      if (!agentIds().includes(o)) dieZh("handle", id, `--owner ${o} 不是已登记角色`, w.status, 2);
      owner = o;
      mode = "owner";
    }
    const from = w.owner;
    w.owner = owner;
    w.progress = mode === "intake" ? `已转交 intake ${owner}（ops/工具类）` : mode === "owner" ? `已指定负责人 ${owner}` : `由 ${as} 自己处理`;
    hist(w, "handle", mode === "intake" ? `转交 intake ${owner}` : mode === "owner" ? `指定 owner ${owner}` : "上级自己处理", { to: owner });
    commit(id, `${id}: handle owner ${from} -> ${owner} (${mode})`, { from, to: owner, mode });
    break;
  }
  case "forward": {
    const id = opt("--id"); const w = need(id); const note = opt("--note");
    if (w.kind !== "request") dieZh("forward", id, "不是请求包", w.status);
    requireActor(approverOf(w), "审批人", id);
    if (w.status !== "awaiting_approval") dieZh("forward", id, "只有待审批才能继续上交", w.status);
    const cfg = opsConfigOf(board);
    const next = superiorOfAgent(board, as);
    if (!next) dieZh("forward", id, `${as} 没有更上一级`, w.status);
    if (next === cfg.principal && !cfg.escalate_to_principal) {
      dieZh("forward", id, "上交到此为止：配置不允许把请求直接交给委托人", w.status);
    }
    w.approver = next; w.reviewer = next;
    w.progress = `已上交给 ${next}`;
    hist(w, "forward", note, { to: next });
    const ev = emit("team_ops.approval_requested", `${id} 由 ${as} 上交给 ${next}`, id, {
      stage: "plan", approver: next, requester: w.requester, forwarded_by: as, path: wpChain(board, id),
    });
    commit(id, `${id}: forward approver → ${next}`, { event: ev, approver: next });
    break;
  }
  case "org-tree": {
    const rows = orgTreeRows(board);
    const derived = deriveReporting(board);
    if (flag("--json")) {
      console.log(JSON.stringify({
        ok: true,
        rows,
        reporting: Object.fromEntries([...derived.sup.entries()].map(([k, v]) => [k, { superior: v, subordinates: derived.subs.get(k) || [] }])),
        ops_config: opsConfigOf(board),
      }, null, 2));
    } else {
      for (const r of rows) {
        const pad = "  ".repeat(r.depth);
        console.log(`${pad}${r.depth ? "└ " : ""}${r.id} [${r.name}] lead=${r.lead} members=${r.members.join(",") || "-"}`);
      }
      console.log("--- reporting ---");
      for (const a of board.agents) {
        const sup = derived.sup.get(a.id) ?? a.superior ?? "-";
        const subs = (derived.subs.get(a.id) || a.subordinates || []).join(",") || "-";
        console.log(`${a.id}: superior=${sup} subordinates=${subs}`);
      }
    }
    recordTrace(ops, null, TOOL, reg.tools_version);
    break;
  }
  case "org-sync": {
    if (as !== "gm" && as !== "andy") dieZh("org-sync", null, "只有 gm/andy 能 org-sync");
    if (!(board.org_nodes || []).length) dieZh("org-sync", null, "board 没有 org_nodes");
    applyDerivedReporting(board);
    board.updated_at = nowIso;
    commit(null, `org-sync: rewrote superior/subordinates for ${board.agents.length} agents`, {
      agents: board.agents.map((a) => ({ id: a.id, node: a.node, superior: a.superior, subordinates: a.subordinates })),
    });
    break;
  }
  case "role-charter": {
    const id = opt("--id");
    if (!id) dieZh("role-charter", null, "需要 --id <agent>", null, 2);
    let constitutionText = null;
    const cPath = path.join(ops, "context", "constitution.md");
    try { constitutionText = fs.readFileSync(cPath, "utf8"); } catch { /* placeholder text inside charterOf */ }
    const out = charterOf(board, id, { constitutionText });
    if (!out) dieZh("role-charter", id, "没有这个角色");
    if (flag("--json")) console.log(JSON.stringify({ ok: true, ...out }, null, 2));
    else console.log(out.markdown);
    recordTrace(ops, null, TOOL, reg.tools_version);
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
    const id = opt("--id"); if (!id) dieZh("summary", null, "需要 --id", null, 2); const w = need(id);
    const kids = wpTreeOrder(board, id).slice(1).map(({ w: c, depth }) => ({ depth, id: c.id, status: c.status, owner: c.owner, approver: approverOf(c), artifact: c.artifact_path ?? null, last: (c.history || []).slice(-1)[0] ?? null }));
    const out = { ok: true, id, status: w.status, approver: approverOf(w), approved: isApproved(w), path: wpChain(board, id), content_kind: w.content?.kind ?? null, kind: w.kind ?? null, requester: w.requester ?? null,
      children: kids, open_children: kids.filter((k) => !CLOSED_STATUSES.has(k.status)).map((k) => k.id), last: (w.history || []).slice(-1)[0] ?? null };
    if (flag("--json")) console.log(JSON.stringify(out, null, 2));
    else {
      console.log(`${id} [${w.status}] approver=${out.approver} approved=${out.approved} path=${out.path.join(" > ")} kind=${out.kind ?? out.content_kind ?? "-"}`);
      for (const k of kids) console.log(`${"  ".repeat(k.depth)}- ${k.id} [${k.status}] owner=${k.owner} artifact=${k.artifact ?? "-"} last=${k.last ? k.last.action + "@" + k.last.actor : "-"}`);
      console.log(`open children: ${out.open_children.join(",") || "none"}`);
    }
    recordTrace(ops, id, TOOL, reg.tools_version);
    break;
  }
  case "history": {
    const id = opt("--id"); if (!id) dieZh("history", null, "需要 --id", null, 2); const w = need(id);
    const h = w.history || [];
    if (flag("--json")) console.log(JSON.stringify({ ok: true, id, history: h }, null, 2));
    else for (const e of h) console.log(`${e.ts} ${e.actor} ${e.action}${e.to ? "→" + e.to : ""}${e.note ? "  " + e.note : ""}`);
    recordTrace(ops, id, TOOL, reg.tools_version);
    break;
  }
}

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { orgProblems } from "./tools/_lib.mjs";

const dir = path.dirname(fileURLToPath(import.meta.url));
const board = JSON.parse(fs.readFileSync(path.join(dir, "board.json"), "utf8"));
const schema = JSON.parse(fs.readFileSync(path.join(dir, "board.schema.json"), "utf8"));

function fail(msg) {
  console.error("INVALID:", msg);
  process.exit(1);
}

function typeOf(v) {
  if (v === null) return "null";
  if (Array.isArray(v)) return "array";
  return typeof v;
}

function check(node, sch, at) {
  if (sch.const !== undefined && node !== sch.const) fail(at + ": expected const " + sch.const);
  if (sch.enum && !sch.enum.includes(node)) fail(at + ": not in enum");
  if (sch.type) {
    const types = Array.isArray(sch.type) ? sch.type : [sch.type];
    const t = typeOf(node);
    const ok = types.some((x) =>
      x === "integer" ? Number.isInteger(node) : x === t || (x === "number" && typeof node === "number")
    );
    if (!ok) fail(at + ": type " + t + ", want " + types.join("|"));
  }
  if (sch.pattern && typeof node === "string" && !new RegExp(sch.pattern).test(node)) fail(at + ": pattern");
  if (sch.minLength != null && typeof node === "string" && node.length < sch.minLength) fail(at + ": minLength");
  if (sch.minItems != null && Array.isArray(node) && node.length < sch.minItems) fail(at + ": minItems");
  if (sch.additionalProperties === false && node && typeof node === "object" && !Array.isArray(node)) {
    for (const k of Object.keys(node)) {
      if (!sch.properties || !(k in sch.properties)) fail(at + ": unexpected key " + k);
    }
  }
  if (sch.required && node && typeof node === "object") {
    for (const k of sch.required) if (!(k in node) || node[k] === undefined) fail(at + ": missing " + k);
  }
  if (sch.properties && node && typeof node === "object") {
    for (const [k, v] of Object.entries(node)) {
      if (sch.properties[k]) check(v, sch.properties[k], at + "." + k);
    }
  }
  if (sch.items && Array.isArray(node)) {
    node.forEach((it, i) => check(it, sch.items, at + "[" + i + "]"));
  }
}

check(board, schema, "board");

const agentIds = new Set(board.agents.map((a) => a.id));
const humanIds = new Set(["andy"]);
const wpIds = new Set(board.work_packages.map((w) => w.id));
for (const a of board.agents) {
  if (a.kind !== "human" && !a.place.startsWith("/workspace/litecode")) {
    fail("agent " + a.id + ": place must be inside /workspace/litecode (cloud repo contract)");
  }
}
const domainIds = new Set(board.domains.map((d) => d.id));
for (const w of board.work_packages) {
  if (!domainIds.has(w.domain)) fail("WP " + w.id + ": domain " + JSON.stringify(w.domain) + " not in board.domains"); // tools_version 17
  for (const d of w.depends_on) if (!wpIds.has(d)) fail("WP " + w.id + " depends_on missing " + d);
  if (w.owner && !agentIds.has(w.owner)) fail("WP " + w.id + " owner unknown");
  if (w.reviewer && !agentIds.has(w.reviewer) && !humanIds.has(w.reviewer)) {
    fail("WP " + w.id + " reviewer unknown");
  }
  if (w.blocker && w.blocker.kind === "other" && w.status === "blocked") {
    fail("WP " + w.id + ": other blockers must not block pipeline status=blocked");
  }
}
// v13: nested WP model checks (WP-020)
const byId = new Map(board.work_packages.map((w) => [w.id, w]));
const CLOSED = new Set(["done", "cancelled", "blocked"]);
for (const w of board.work_packages) {
  if (w.parent !== undefined && w.parent !== null) {
    if (w.parent === w.id) fail("WP " + w.id + ": parent points to itself");
    if (!byId.has(w.parent)) fail("WP " + w.id + ": parent missing " + w.parent);
    const seen = new Set([w.id]);
    let cur = byId.get(w.parent);
    while (cur) {
      if (seen.has(cur.id)) fail("WP " + w.id + ": parent cycle via " + cur.id);
      seen.add(cur.id);
      cur = cur.parent ? byId.get(cur.parent) : null;
    }
  }
  if (w.approver !== undefined && w.approver !== null && !agentIds.has(w.approver) && !humanIds.has(w.approver)) {
    fail("WP " + w.id + " approver unknown");
  }
  if (w.status === "done") {
    const kids = board.work_packages.filter((c) => c.parent === w.id);
    const open = kids.filter((c) => c.status !== "done" && c.status !== "cancelled" && c.status !== "blocked");
    if (open.length) fail("WP " + w.id + ": done but children not closed: " + open.map((c) => c.id).join(","));
    const bad = kids.filter((c) => c.status === "blocked");
    if (bad.length) {
      const appr = w.approver ?? w.reviewer;
      const h = Array.isArray(w.history) ? w.history : [];
      if (!h.some((e) => e.action === "close" && e.actor === appr && e.note)) {
        fail("WP " + w.id + ": done with blocked children requires a close by approver with note");
      }
    }
  }
}
for (const p of board.pipelines) {
  if (!agentIds.has(p.owner)) fail("pipeline " + p.id + " owner");
  if (!agentIds.has(p.reviewer) && !humanIds.has(p.reviewer)) fail("pipeline " + p.id + " reviewer");
  for (const id of p.work_packages) if (!wpIds.has(id)) fail("pipeline " + p.id + " missing WP " + id);
}

// WP-189: org tree + request kind
for (const msg of orgProblems(board)) fail(msg);
for (const w of board.work_packages) {
  if (w.kind === "request") {
    if (!w.requester) fail("WP " + w.id + ": kind=request 必须有 requester");
    else if (!agentIds.has(w.requester) && !humanIds.has(w.requester)) fail("WP " + w.id + " requester unknown");
  }
}

console.log("ops board OK", {
  domains: board.domains.length,
  agents: board.agents.length,
  work_packages: board.work_packages.length,
  pipelines: board.pipelines.length,
});
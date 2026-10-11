// ops.kb_check — knowledge base checker for team-ops (tools_version >= 7).
// Parsing / reference resolution is a Node port of litecode (kb_parse.mjs, kb_scopes.mjs),
// locked by tests/fixtures/knowledge_refs/cases.json (kb_check.test.mjs / --selftest).
// On top of litecode's own verdicts it adds the admission rules of
// .litecode/ops/context/knowledge-node-admission.md (§0, §5, §6).
//
// CLI: node .litecode/ops/tools/kb_check.mjs [--node <key>] [--json] [--wp WP-xxx]
//                                            [--stock-ref <rev>] [--selftest]
// Exit: 0 no errors (warnings allowed) · 1 errors · 2 usage / unknown node.
import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { loadRegistry, printVersionHeader, opsRoot, recordTrace } from "./_lib.mjs";
import { computeOnce, scanProse, isKnowledgeKey, normalizeKey } from "./kb_parse.mjs";

export const TOOL_ID = "ops.kb_check";
const here = fileURLToPath(import.meta.url);
const OPS = opsRoot(import.meta.url);
export const REPO = path.resolve(OPS, "..", "..");

/** Five relation tags (admission §5); X展开 is one tag in three spellings. 证据 is §7.1's template spelling of 依据. */
export const ALLOWED_TAGS = ["上层", "父节点", "契约展开", "框架展开", "向下展开", "相关", "依据", "证据"];
const PARENT_TAGS = new Set(["上层", "父节点"]);

/** litecode wire codes and their kb_check severity (ops policy). */
const LITECODE_SEVERITY = {
  dangling_relation: "error", self_relation: "error", inactive_target: "warning", missing_file: "error",
  symlink_escape: "error", invalid_lines: "error", unreadable_file: "error", unsupported_language: "error",
  missing_symbol: "error", ambiguous_symbol: "error", symbol_drift: "warning", line_out_of_range: "error",
  invalid_status: "error", empty_key: "error", duplicate_key: "error",
  filename_mismatch: "error", // litecode: warning; admission §0 "文件名主干必须等于键" -> error here
};

const REASONS = {
  no_space_after_at: "`[@` 后必须恰好一个空格",
  space_before_at: "`[` 与 `@` 之间不能有空格",
  extra_space: "属性之间只能有一个空格（`[@ ` 后、属性间、`]` 前都不能多空格）",
  fullwidth_bracket: "用了全角括号（［ 或 【），只认半角 `[@ … ]`",
  bad_quotes: "属性值必须用半角双引号 \"…\"（不认中文引号/单引号/无引号）",
  unknown_attribute: "出现未知属性（只认 key / file / symbol / lines）",
  mixed_attributes: "`key` 不能与 file/symbol/lines 混用",
  attribute_order: "属性顺序必须是 file → symbol → lines",
  missing_file_attr: "symbol/lines 引用缺少 file 属性",
  attr_spacing: "属性写法必须是 name=\"value\"（等号两侧不能有空格）",
  invalid_key: "key 不满足键规则（字母/数字/_/-，词之间单个空格）",
  empty_key: "key 为空",
  empty_path: "file 为空",
  unclosed: "缺少收尾的 `]`",
  not_recognized: "不是 litecode 认得的四种写法之一",
};

function readText(p) { try { return fs.readFileSync(p, "utf8"); } catch { return null; } }

function boardDomains() {
  const b = JSON.parse(fs.readFileSync(path.join(OPS, "board.json"), "utf8"));
  return { ids: b.domains.map((d) => d.id), board: b };
}
/** Stock nodes = node files tracked at the admission doc's baseline commit (§1 盘点基线). */
export function stockSet(ref) {
  let source = "--stock-ref";
  if (!ref) {
    const doc = readText(path.join(OPS, "context", "knowledge-node-admission.md")) || "";
    const m = /基线提交：`([0-9a-f]{7,40})`/.exec(doc);
    ref = m ? m[1] : "HEAD"; source = m ? "knowledge-node-admission.md 基线提交" : "HEAD (admission doc has no 基线提交)";
  }
  const r = spawnSync("git", ["ls-tree", "-r", "--name-only", ref, "--", "knowledge/"], { cwd: REPO, encoding: "utf8" });
  if (r.status !== 0) return { ref, source, ok: false, paths: null };
  return { ref, source, ok: true, paths: new Set(r.stdout.split("\n").filter(Boolean)) };
}

// ------------------------------------------------------------ malformed citations

function diagnose(snippet) {
  if (/^[［【]/.test(snippet)) return "fullwidth_bracket";
  if (/^\[\s+@/.test(snippet)) return "space_before_at";
  if (/^\[@(?! )/.test(snippet)) return "no_space_after_at";
  if (!snippet.includes("]")) return "unclosed";
  if (/^\[@ {2,}/.test(snippet) || /" {2,}\w/.test(snippet) || /"\s+\]$/.test(snippet) || /\s\]$/.test(snippet)) return "extra_space";
  if (/[“”‘’「」]/.test(snippet) || /=\s*'/.test(snippet) || /=[^"\s\]]/.test(snippet)) return "bad_quotes";
  if (/\w\s+=|=\s+"/.test(snippet)) return "attr_spacing";
  const names = [...snippet.matchAll(/([A-Za-z_]+)="([^"]*)"/g)].map((m) => [m[1], m[2]]);
  const n = names.map((x) => x[0]);
  if (n.some((x) => !["key", "file", "symbol", "lines"].includes(x))) return "unknown_attribute";
  if (n.includes("key") && n.length > 1) return "mixed_attributes";
  if (n.includes("key")) { const k = normalizeKey(names[0][1]); return k === "" ? "empty_key" : !isKnowledgeKey(k) ? "invalid_key" : "not_recognized"; }
  if (!n.includes("file") && (n.includes("symbol") || n.includes("lines"))) return "missing_file_attr";
  const order = n.map((x) => ["file", "symbol", "lines"].indexOf(x));
  if (order.some((v, i) => i > 0 && v <= order[i - 1])) return "attribute_order";
  if (n.includes("file") && normalizeKey(names.find((x) => x[0] === "file")[1]) === "") return "empty_path";
  return "not_recognized";
}

const SUSPECT = /\[\s*@|[［【]\s*@|\[\[\s*node\s*:|(?<![\[［【]\s*)@\s?(?:key|file|symbol)\s*=/g;
const VALID_NODE = /\[@ key="([^"]*)"\]/g;
const VALID_FILE = /\[@ file="([^"]*)"(?: symbol="([^"]*)")?(?: lines="([^"]*)")?\]/g;

function malformedIn(body, bodyStartLine) {
  const out = [];
  for (const { masked, lineNo, line, map } of scanProse(body)) {
    if (!/[@［【]|\[\[/.test(masked)) continue;
    const spans = [];
    for (const m of masked.matchAll(VALID_NODE)) { const k = normalizeKey(m[1]); if (k && isKnowledgeKey(k)) spans.push([m.index, m.index + m[0].length]); }
    for (const m of masked.matchAll(VALID_FILE)) if (normalizeKey(m[1]) !== "") spans.push([m.index, m.index + m[0].length]);
    for (const m of masked.matchAll(SUSPECT)) {
      const at = m.index;
      if (spans.some(([s, e]) => at >= s && at < e)) continue;
      const close = /^\[\[/.test(m[0]) ? masked.indexOf("]]", at) + 1 : masked.indexOf("]", at);
      const end = Math.min(close < 0 ? masked.length : close + 1, at + 160);
      const snippet = line.slice(map[at], map[end] ?? line.length);
      if (/^\[\[\s*node\s*:/.test(m[0])) { out.push({ code: "ref_legacy", line: bodyStartLine + lineNo - 1, snippet, reason: "legacy" }); continue; }
      const reason = m[0].startsWith("@") ? "not_recognized" : diagnose(masked.slice(at, end));
      out.push({ code: "ref_malformed", line: bodyStartLine + lineNo - 1, snippet, reason });
    }
  }
  return out;
}

// ------------------------------------------------------------ tags / parent / baseline

const TAG_RE = /(?:^|[\s>*（(。；;，,、：:])(?:\*\*)?([\p{Script=Han}]{1,10})(?:\*\*)?[：:](?:\*\*)?/gu;
function bodyStructure(body, bodyStartLine) {
  const tags = []; // {tag, line, nodeCites, followedByCite}
  let baseline = null;
  for (const { masked, lineNo } of scanProse(body)) {
    const ms = [...masked.matchAll(TAG_RE)];
    ms.forEach((m, idx) => {
      const tagEnd = m.index + m[0].length;
      const segEnd = idx + 1 < ms.length ? ms[idx + 1].index : masked.length;
      const seg = masked.slice(tagEnd, segEnd);
      const followedByCite = /^\s*\[@ /.test(seg);
      const nodeCites = [...seg.matchAll(VALID_NODE)].filter((x) => isKnowledgeKey(normalizeKey(x[1]))).length;
      tags.push({ tag: m[1], line: bodyStartLine + lineNo - 1, nodeCites, followedByCite });
    });
    const b = /(?:^|\s)基线[：:]\s*`?([0-9a-fA-F]*)`?/.exec(masked);
    if (b) baseline = { hash: b[1], line: bodyStartLine + lineNo - 1 };
  }
  return { tags, baseline };
}
function domainOf(extras) {
  for (const e of extras) { const m = /^domain\s*:\s*(.*?)\s*$/.exec(e); if (m) return m[1]; }
  return null;
}

// ------------------------------------------------------------ main check

/**
 * Run the whole corpus. Returns {meta, nodes:[{key,path,status,stock}], findings:[...]}.
 * Each finding: {severity, code, node, path, line?, message, ref_key?, source, args?}.
 */
export function kbCheck(opts = {}) {
  const workspace = opts.workspace || REPO;
  const content = computeOnce(workspace);
  const root = content.root;
  const { ids: domainIds } = boardDomains();
  const stock = stockSet(opts.stockRef);
  const findings = [];
  const nodesOut = [];
  const isStock = (n) => (stock.ok ? stock.paths.has(`${root}/${n.path}`) : true);
  const add = (n, f) => findings.push({ node: n ? n.key || n.path : null, path: n ? `${root}/${n.path}` : f.path, ...f });

  for (const n of content.nodes) {
    const st = isStock(n);
    nodesOut.push({ key: n.key, path: `${root}/${n.path}`, status: n.invalid_status ?? n.status, stock: st, domain: domainOf(n.extras) });
    // 1. litecode verdicts (same codes as GET /api/workspace/knowledge)
    for (const i of n.issues) {
      let code = i.code, severity = LITECODE_SEVERITY[code] || i.severity;
      let message = i.message;
      if ((code === "missing_symbol" || code === "ambiguous_symbol") && i.meta && i.meta.approx) {
        severity = "warning"; message = `${message} (kb_check 对该语言只做近似解析，未能确认：${code})`; code = "symbol_unverified";
      }
      const ref = n.refs.find((r) => r.ref_key === i.ref_key);
      add(n, { severity, code, line: ref && ref.lineNo ? n.bodyStartLine + ref.lineNo - 1 : undefined, message, ref_key: i.ref_key ? i.ref_key.replace("\0", " :: ") : undefined, raw: ref ? ref.raw : undefined, source: "litecode" });
    }
    // 2. look-alike citations litecode silently treats as text
    for (const m of malformedIn(n.body, n.bodyStartLine)) {
      add(n, { severity: "error", code: m.code, line: m.line, message: m.code === "ref_legacy" ? `旧写法 [[node : K]] 不是引用（KR-SYN-7），改成 [@ key="K"]：${m.snippet}` : `看起来像引用、但 litecode 会当成普通文本（${REASONS[m.reason] || m.reason}）：${m.snippet}`, args: { reason: m.reason }, source: "ops" });
    }
    if (n.summary.includes("[@") || n.extras.some((e) => e.includes("[@"))) {
      add(n, { severity: "warning", code: "ref_in_declaration", message: "声明块（summary/其它字段）里的 [@ …] 不是引用（引用只在声明块之后的正文里），不会被解析", source: "ops" });
    }
    // 3. new-node rules (admission §5/§6/§7); stock nodes -> warning only
    const sev = st ? "warning" : "error";
    const tagNote = st ? "（存量节点，仅提示）" : "";
    const dom = domainOf(n.extras);
    if (dom === null) add(n, { severity: sev, code: "domain_missing", message: `声明块缺少 domain : <域 id> 行${tagNote}`, source: "ops" });
    else if (!domainIds.includes(dom) || dom === "team-ops") add(n, { severity: sev, code: "domain_unknown", message: `domain "${dom}" 不在 board.json 的产品域里（可选：${domainIds.filter((d) => d !== "team-ops").join(", ")}）${tagNote}`, args: { domain: dom }, source: "ops" });
    const { tags, baseline } = bodyStructure(n.body, n.bodyStartLine);
    const parents = tags.filter((t) => PARENT_TAGS.has(t.tag));
    const parentRefs = parents.reduce((a, t) => a + t.nodeCites, 0);
    if (parents.length === 0 || parentRefs === 0) add(n, { severity: sev, code: "parent_missing", message: `缺少 上层：[@ key="…"] 或 父节点：[@ key="…"]（二选一，恰好一个）${tagNote}`, source: "ops" });
    else if (parents.length > 1 || parentRefs > 1) add(n, { severity: sev, code: "parent_multiple", line: parents[0].line, message: `上层/父节点 应恰好一个，实际 ${parents.map((t) => t.tag).join("+")} 共引用 ${parentRefs} 个节点${tagNote}`, args: { tags: parents.map((t) => t.tag), refs: parentRefs }, source: "ops" });
    if (!baseline) add(n, { severity: sev, code: "baseline_missing", message: `正文缺少 基线：<短哈希> 行${tagNote}`, source: "ops" });
    else if (!/^[0-9a-f]{7,40}$/.test(baseline.hash)) add(n, { severity: sev, code: "baseline_missing", line: baseline.line, message: `基线：后不是短哈希（7–40 位小写十六进制）${tagNote}`, source: "ops" });
    for (const t of tags) {
      if (t.followedByCite && !ALLOWED_TAGS.includes(t.tag)) add(n, { severity: sev, code: "tag_unknown", line: t.line, message: `关系标签「${t.tag}：」不在规定的五个里（上层/父节点/契约展开·框架展开·向下展开/相关/依据）${tagNote}`, args: { tag: t.tag }, source: "ops" });
    }
    if (!st && n.status !== "pending") add(n, { severity: "error", code: "new_not_pending", message: `新节点必须 status : pending（状态升降由 Andy 决定），当前 ${n.invalid_status ?? n.status}`, source: "ops" });
  }

  // 4. suspected duplicate main nodes: same symbol/range cited as evidence by >1 node, or same summary
  const bySym = new Map();
  for (const n of content.nodes) for (const r of n.refs) if (r.kind === "symbol" || r.kind === "range") {
    const k = r.ref_key.replace("\0", " :: ");
    if (!bySym.has(k)) bySym.set(k, new Set());
    bySym.get(k).add(n);
  }
  const pair = new Map(); // key -> Map(otherKey -> [symbols])
  for (const [sym, set] of bySym) if (set.size > 1) for (const a of set) for (const b of set) if (a !== b) {
    if (!pair.has(a)) pair.set(a, new Map());
    const m = pair.get(a);
    if (!m.has(b)) m.set(b, []);
    m.get(b).push(sym);
  }
  for (const [a, m] of pair) for (const [b, syms] of m) {
    add(a, { severity: "warning", code: "dup_primary_suspect", message: `与 ${b.key} 用同一份源码证据（${syms.length} 处：${syms.slice(0, 3).join("；")}${syms.length > 3 ? "；…" : ""}），疑似主节点重复——一个契约只有一个主节点，另一处应改为 [@ key="…"] 引用`, args: { other: b.key, shared: syms }, source: "ops" });
  }
  const bySummary = new Map();
  for (const n of content.nodes) { const s = n.summary.trim(); if (!s) continue; if (!bySummary.has(s)) bySummary.set(s, []); bySummary.get(s).push(n); }
  for (const [s, list] of bySummary) if (list.length > 1) for (const a of list) {
    add(a, { severity: "warning", code: "dup_primary_suspect", message: `summary 与 ${list.filter((x) => x !== a).map((x) => x.key).join(", ")} 完全相同，疑似主节点重复`, args: { other: list.filter((x) => x !== a).map((x) => x.key), summary: s }, source: "ops" });
  }
  // 5. .md files in the knowledge root that are not nodes
  for (const u of content.unknown) findings.push({ severity: "warning", code: "not_a_node", node: null, path: `${root}/${u.path}`, message: "知识根下的 .md 没有 node 声明块（或键为空），litecode 不把它当节点", source: "ops" });

  const order = { error: 0, warning: 1 };
  findings.sort((x, y) => order[x.severity] - order[y.severity] || String(x.path).localeCompare(String(y.path)) || (x.line || 0) - (y.line || 0));
  return {
    meta: { root, head: content.head, nodes: content.nodes.length, stock_ref: stock.ref, stock_source: stock.source, stock_ok: stock.ok, stock_nodes: nodesOut.filter((n) => n.stock).length, new_nodes: nodesOut.filter((n) => !n.stock).length, refs: content.nodes.reduce((a, n) => a + n.refs.length, 0) },
    nodes: nodesOut,
    findings,
  };
}

export function summarize(findings) {
  const by = {};
  for (const f of findings) { const k = `${f.severity}:${f.code}`; by[k] = (by[k] || 0) + 1; }
  return { errors: findings.filter((f) => f.severity === "error").length, warnings: findings.filter((f) => f.severity === "warning").length, by_code: by };
}

/** Findings that belong to the given node keys / paths. */
export function findingsFor(result, keys) {
  const want = new Set(keys);
  const nodes = result.nodes.filter((n) => want.has(n.key) || want.has(n.path) || want.has(n.path.replace(/^knowledge\//, "")));
  const paths = new Set(nodes.map((n) => n.path));
  return { nodes, findings: result.findings.filter((f) => paths.has(f.path)), unknown: [...want].filter((k) => !nodes.some((n) => n.key === k || n.path === k || n.path === `knowledge/${k}`)) };
}

// ------------------------------------------------------------ CLI

function main() {
  const { reg } = loadRegistry(import.meta.url);
  const args = process.argv.slice(2);
  const arg = (n) => { const i = args.indexOf(n); return i >= 0 ? args[i + 1] : null; };
  const json = args.includes("--json");
  printVersionHeader(reg, TOOL_ID);
  const known = new Set(["--node", "--json", "--wp", "--stock-ref", "--selftest"]);
  for (let i = 0; i < args.length; i++) {
    if (!known.has(args[i])) { console.error(`unknown argument ${args[i]}\nusage: kb_check.mjs [--node <key>] [--json] [--wp WP-xxx] [--stock-ref <rev>] [--selftest]`); process.exit(2); }
    if (["--node", "--wp", "--stock-ref"].includes(args[i])) { if (!args[i + 1] || args[i + 1].startsWith("--")) { console.error(`${args[i]} needs a value`); process.exit(2); } i++; }
  }
  const wp = arg("--wp"), nodeKey = arg("--node");
  if (args.includes("--selftest")) {
    const r = spawnSync(process.execPath, [path.join(path.dirname(here), "kb_check.test.mjs"), ...(json ? ["--json"] : [])], { encoding: "utf8" });
    process.stdout.write(r.stdout || ""); process.stderr.write(r.stderr || "");
    recordTrace(OPS, wp, TOOL_ID, reg.tools_version);
    process.exit(r.status ?? 1);
  }
  const result = kbCheck({ stockRef: arg("--stock-ref") });
  let findings = result.findings, scope = "all";
  if (nodeKey) {
    const sel = findingsFor(result, [nodeKey]);
    if (sel.nodes.length === 0) { console.error(`kb_check: no node with key or path "${nodeKey}"`); process.exit(2); }
    findings = sel.findings; scope = sel.nodes.map((n) => n.path).join(",");
  }
  const sum = summarize(findings);
  recordTrace(OPS, wp, TOOL_ID, reg.tools_version);
  const out = { ok: sum.errors === 0, tools_version: reg.tools_version, scope, ...result.meta, ...sum, findings };
  if (json) { console.log(JSON.stringify(out, null, 2)); process.exit(out.ok ? 0 : 1); }
  const m = result.meta;
  console.log(`kb_check: ${m.nodes} nodes (stock ${m.stock_nodes}, new ${m.new_nodes}; stock_ref=${m.stock_ref}) · ${m.refs} refs · root=${m.root} · head=${(m.head || "-").slice(0, 7)} · scope=${scope}`);
  console.log(`errors=${sum.errors} warnings=${sum.warnings}`);
  for (const [k, v] of Object.entries(sum.by_code).sort((a, b) => b[1] - a[1])) console.log(`  ${k} ×${v}`);
  for (const f of findings) console.log(`[${f.severity === "error" ? "E" : "W"}] ${f.code}  ${f.node ?? "-"}  ${f.path}${f.line ? ":" + f.line : ""}  ${f.message}${f.ref_key ? "  <" + f.ref_key + ">" : ""}`);
  console.log(JSON.stringify({ ok: out.ok, tools_version: reg.tools_version, nodes: m.nodes, scope, errors: sum.errors, warnings: sum.warnings }));
  process.exit(out.ok ? 0 : 1);
}

if (process.argv[1] && path.resolve(process.argv[1]) === here) main();

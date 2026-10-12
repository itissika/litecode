// ops.kb_nav — citation graph navigation for team-ops (tools_version >= 12).
// Reuses kb_parse.mjs (computeOnce / extractCites); parent tags match kb_check admission.
//
// CLI: node .litecode/ops/tools/kb_nav.mjs --key <key> [--json] [--wp WP-xxx]
//      node .litecode/ops/tools/kb_nav.mjs --orphans [--json] [--wp WP-xxx]
//      node .litecode/ops/tools/kb_nav.mjs --selftest [--json]
// Exit: 0 ok · 1 missing key · 2 usage.
import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { loadRegistry, printVersionHeader, opsRoot, recordTrace } from "./_lib.mjs";
import { computeOnce, scanProse, isKnowledgeKey, normalizeKey } from "./kb_parse.mjs";
import { stockSet } from "./kb_check.mjs";

export const TOOL_ID = "ops.kb_nav";
const here = fileURLToPath(import.meta.url);
const OPS = opsRoot(import.meta.url);
export const REPO = path.resolve(OPS, "..", "..");

const PARENT_TAGS = new Set(["上层", "父节点"]);
const TAG_RE = /(?:^|[\s>*（(。；;，,、：:])(?:\*\*)?([\p{Script=Han}]{1,10})(?:\*\*)?[：:](?:\*\*)?/gu;
const VALID_NODE = /\[@ key="([^"]*)"\]/g;

/** Detect 上层/父节点 cites and baseline line in body (same rules as kb_check). */
export function parentInfo(body, bodyStartLine = 1) {
  const parents = [];
  let baseline = null;
  for (const { masked, lineNo } of scanProse(body)) {
    const ms = [...masked.matchAll(TAG_RE)];
    ms.forEach((m, idx) => {
      if (!PARENT_TAGS.has(m[1])) return;
      const tagEnd = m.index + m[0].length;
      const segEnd = idx + 1 < ms.length ? ms[idx + 1].index : masked.length;
      const seg = masked.slice(tagEnd, segEnd);
      const keys = [...seg.matchAll(VALID_NODE)]
        .map((x) => normalizeKey(x[1]))
        .filter((k) => k && isKnowledgeKey(k));
      parents.push({ tag: m[1], line: bodyStartLine + lineNo - 1, keys });
    });
    const b = /(?:^|\s)基线[：:]\s*`?([0-9a-fA-F]*)`?/.exec(masked);
    if (b) baseline = { hash: b[1], line: bodyStartLine + lineNo - 1 };
  }
  const parentKeys = parents.flatMap((p) => p.keys);
  return { parents, parentKeys, baseline, parentCount: parentKeys.length };
}

function summarizeFileCites(cites) {
  const files = [], symbols = [], ranges = [];
  for (const c of cites) {
    if (c.kind === "file") files.push({ path: c.path, line: c.lineNo });
    else if (c.kind === "symbol") symbols.push({ path: c.path, symbol: c.symbol, line: c.lineNo });
    else if (c.kind === "range") ranges.push({ path: c.path, lines: c.lines, line: c.lineNo });
  }
  return { files, symbols, ranges, file_count: files.length, symbol_count: symbols.length, range_count: ranges.length };
}

/** Build outbound/inbound maps from a computeOnce result. */
export function buildGraph(content) {
  const byKey = new Map();
  for (const n of content.nodes) {
    if (!byKey.has(n.key)) byKey.set(n.key, []);
    byKey.get(n.key).push(n);
  }
  const outbound = new Map(); // key -> [{to, line, raw}]
  const inbound = new Map();  // key -> [{from, path, line, raw}]
  for (const n of content.nodes) {
    const outs = [];
    for (const c of n.cites) {
      if (c.kind !== "node") continue;
      outs.push({ to: c.key, line: n.bodyStartLine + (c.lineNo || 1) - 1, raw: c.raw });
      if (!inbound.has(c.key)) inbound.set(c.key, []);
      inbound.get(c.key).push({ from: n.key, path: `${content.root}/${n.path}`, line: n.bodyStartLine + (c.lineNo || 1) - 1, raw: c.raw });
    }
    if (!outbound.has(n.key)) outbound.set(n.key, []);
    outbound.get(n.key).push(...outs);
  }
  return { byKey, outbound, inbound };
}

export function listOrphans(content, graph) {
  const { inbound } = graph;
  return content.nodes
    .filter((n) => !(inbound.get(n.key) || []).some((e) => e.from !== n.key))
    .map((n) => ({
      key: n.key,
      path: `${content.root}/${n.path}`,
      status: n.invalid_status ?? n.status,
      inbound: (inbound.get(n.key) || []).length,
    }))
    .sort((a, b) => (a.key < b.key ? -1 : a.key > b.key ? 1 : 0));
}

/**
 * Navigate one key. Returns structured report.
 * stockRoot: stock nodes with no parent are still flagged, but tagged stock_root_hint.
 */
export function navigateKey(workspace, key, opts = {}) {
  const content = computeOnce(workspace);
  if (!content.root) return { ok: false, error: "no knowledge root", key };
  const graph = buildGraph(content);
  const nodes = graph.byKey.get(key);
  if (!nodes || nodes.length === 0) {
    return { ok: false, error: `no node with key "${key}"`, key, known: content.nodes.length };
  }
  const stock = opts.stock || stockSet(opts.stockRef);
  const n = nodes[0];
  const pathFull = `${content.root}/${n.path}`;
  const isStock = stock.ok ? stock.paths.has(pathFull) : false;
  const outs = graph.outbound.get(key) || [];
  const inns = (graph.inbound.get(key) || []).filter((e) => e.from !== key);
  const knownKeys = new Set(content.nodes.map((x) => x.key));
  const missing_targets = [...new Set(outs.map((o) => o.to).filter((t) => !knownKeys.has(t)))];
  const pi = parentInfo(n.body, n.bodyStartLine);
  const parent_missing = pi.parentCount === 0;
  const parent_multiple = pi.parentCount > 1 || pi.parents.length > 1;
  const fileSum = summarizeFileCites(n.cites);
  const orphan = inns.length === 0;
  return {
    ok: true,
    key,
    path: pathFull,
    status: n.invalid_status ?? n.status,
    stock: isStock,
    head: content.head ? content.head.slice(0, 7) : null,
    outbound: outs.map((o) => ({ key: o.to, line: o.line, exists: knownKeys.has(o.to) })),
    inbound: inns.map((e) => ({ key: e.from, path: e.path, line: e.line })),
    outbound_count: outs.length,
    inbound_count: inns.length,
    missing_targets,
    parent: {
      missing: parent_missing,
      multiple: parent_multiple,
      keys: pi.parentKeys,
      tags: pi.parents.map((p) => p.tag),
      stock_root_hint: isStock && parent_missing,
    },
    baseline: pi.baseline,
    orphan,
    file_cites: fileSum,
    duplicates: nodes.length > 1 ? nodes.map((x) => `${content.root}/${x.path}`) : undefined,
  };
}

export function navigateOrphans(workspace, opts = {}) {
  const content = computeOnce(workspace);
  if (!content.root) return { ok: false, error: "no knowledge root" };
  const graph = buildGraph(content);
  const orphans = listOrphans(content, graph);
  return {
    ok: true,
    head: content.head ? content.head.slice(0, 7) : null,
    nodes: content.nodes.length,
    orphan_count: orphans.length,
    orphans,
  };
}

function printHuman(report, mode) {
  if (!report.ok) {
    console.log(`kb_nav: ${report.error}`);
    return;
  }
  if (mode === "orphans") {
    console.log(`kb_nav orphans: ${report.orphan_count}/${report.nodes} nodes · head=${report.head || "-"}`);
    for (const o of report.orphans) console.log(`  orphan  ${o.key}  ${o.path}  status=${o.status}`);
    return;
  }
  console.log(`kb_nav: key=${report.key}  path=${report.path}  status=${report.status}  stock=${report.stock}  head=${report.head || "-"}`);
  console.log(`outbound (${report.outbound_count}):`);
  for (const o of report.outbound) console.log(`  → ${o.key}${o.exists ? "" : "  [MISSING]"}  line ${o.line}`);
  if (report.outbound_count === 0) console.log("  (none)");
  console.log(`inbound (${report.inbound_count}):`);
  for (const i of report.inbound) console.log(`  ← ${i.key}  ${i.path}:${i.line}`);
  if (report.inbound_count === 0) console.log("  (none)  [ORPHAN]");
  console.log("hints:");
  if (report.missing_targets.length) console.log(`  missing_targets: ${report.missing_targets.join(", ")}`);
  if (report.parent.missing) console.log(`  parent_missing: 无 上层/父节点${report.parent.stock_root_hint ? "（存量根节点，仅提示）" : ""}`);
  if (report.parent.multiple) console.log(`  parent_multiple: ${report.parent.keys.join(", ")}`);
  if (!report.parent.missing && !report.parent.multiple) console.log(`  parent: ${report.parent.tags[0] || "上层"} → ${report.parent.keys.join(", ") || "-"}`);
  if (!report.baseline) console.log("  baseline_missing");
  else console.log(`  baseline: ${report.baseline.hash}`);
  if (report.orphan) console.log("  orphan: 无入边");
  const fc = report.file_cites;
  console.log(`file/symbol cites: files=${fc.file_count} symbols=${fc.symbol_count} ranges=${fc.range_count}`);
}

function main() {
  const { reg } = loadRegistry(import.meta.url);
  const args = process.argv.slice(2);
  const arg = (n) => { const i = args.indexOf(n); return i >= 0 ? args[i + 1] : null; };
  const json = args.includes("--json");
  printVersionHeader(reg, TOOL_ID);
  const known = new Set(["--key", "--orphans", "--json", "--wp", "--selftest", "--stock-ref", "--workspace"]);
  for (let i = 0; i < args.length; i++) {
    if (!known.has(args[i])) {
      console.error(`unknown argument ${args[i]}\nusage: kb_nav.mjs --key <key> [--json] [--wp WP-xxx]\n       kb_nav.mjs --orphans [--json] [--wp WP-xxx]\n       kb_nav.mjs --selftest`);
      process.exit(2);
    }
    if (["--key", "--wp", "--stock-ref", "--workspace"].includes(args[i])) {
      if (!args[i + 1] || args[i + 1].startsWith("--")) { console.error(`${args[i]} needs a value`); process.exit(2); }
      i++;
    }
  }
  const wp = arg("--wp");
  if (args.includes("--selftest")) {
    const r = spawnSync(process.execPath, [path.join(path.dirname(here), "kb_nav.test.mjs"), ...(json ? ["--json"] : [])], { encoding: "utf8" });
    process.stdout.write(r.stdout || "");
    process.stderr.write(r.stderr || "");
    recordTrace(OPS, wp, TOOL_ID, reg.tools_version);
    process.exit(r.status ?? 1);
  }
  const workspace = arg("--workspace") || REPO;
  const key = arg("--key");
  const orphans = args.includes("--orphans");
  if (!key && !orphans) { console.error("usage: need --key <key> or --orphans"); process.exit(2); }
  if (key && orphans) { console.error("use --key or --orphans, not both"); process.exit(2); }

  let report;
  if (orphans) report = navigateOrphans(workspace, { stockRef: arg("--stock-ref") });
  else report = navigateKey(workspace, key, { stockRef: arg("--stock-ref") });

  recordTrace(OPS, wp, TOOL_ID, reg.tools_version);
  if (json) {
    console.log(JSON.stringify({ tools_version: reg.tools_version, ...report }, null, 2));
  } else {
    printHuman(report, orphans ? "orphans" : "key");
    console.log(JSON.stringify({
      ok: report.ok,
      tools_version: reg.tools_version,
      ...(orphans
        ? { orphan_count: report.orphan_count, nodes: report.nodes }
        : { key: report.key, outbound: report.outbound_count, inbound: report.inbound_count, missing: (report.missing_targets || []).length, orphan: report.orphan }),
    }));
  }
  process.exit(report.ok ? 0 : 1);
}

if (process.argv[1] && path.resolve(process.argv[1]) === here) main();

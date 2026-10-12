// Selftest for ops.kb_nav + ops.kb_scaffold (tools_version >= 12).
// Builds a tiny fixture corpus under .litecode/ops/.kb_nav_tmp/ and asserts
// outbound/inbound/missing/orphan/parent hints + scaffold parse.
// Usage: node .litecode/ops/tools/kb_nav.test.mjs [--json] [--scaffold-only]
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { navigateKey, navigateOrphans, parentInfo } from "./kb_nav.mjs";
import { buildScaffold, writeScaffold } from "./kb_scaffold.mjs";
import { parseKnowledgeMarkdown, computeOnce } from "./kb_parse.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const ops = path.join(here, "..");
const repo = path.resolve(ops, "..", "..");
const tmpRoot = path.join(ops, ".kb_nav_tmp");
const json = process.argv.includes("--json");
const scaffoldOnly = process.argv.includes("--scaffold-only");

const fails = [];
function ok(cond, msg) { if (!cond) fails.push(msg); }
function section(name) { if (!json) console.log(`## ${name}`); }

function writeTree(root, files) {
  fs.rmSync(root, { recursive: true, force: true });
  fs.mkdirSync(root, { recursive: true });
  for (const [rel, text] of Object.entries(files)) {
    const p = path.join(root, rel);
    fs.mkdirSync(path.dirname(p), { recursive: true });
    fs.writeFileSync(p, text);
  }
}

function nodeMd({ key, status = "enabled", domain = "harness-core", body }) {
  return [
    "```node",
    `node : ${key}`,
    `status : ${status}`,
    `domain : ${domain}`,
    "```",
    "",
    body,
    "",
  ].join("\n");
}

function runNavTests() {
  section("kb_nav fixture corpus");
  const ws = path.join(tmpRoot, "corpus");
  writeTree(ws, {
    "knowledge/root.md": nodeMd({
      key: "root",
      body: "主张：根。\n基线：abcdef1\n契约展开：[@ key=\"child\"]。\n",
    }),
    "knowledge/child.md": nodeMd({
      key: "child",
      body: "主张：子。\n上层：[@ key=\"root\"]。\n相关：[@ key=\"sibling\"]。\n相关：[@ key=\"missing-one\"]。\n基线：abcdef1\n",
    }),
    "knowledge/sibling.md": nodeMd({
      key: "sibling",
      body: "主张：兄。\n上层：[@ key=\"root\"]。\n基线：abcdef1\n",
    }),
    "knowledge/orphan.md": nodeMd({
      key: "orphan",
      status: "pending",
      body: "主张：无人引用。\n上层：[@ key=\"root\"]。\n基线：abcdef1\n",
    }),
  });

  const content = computeOnce(ws);
  ok(content.root === "knowledge", `root should be knowledge, got ${content.root}`);
  ok(content.nodes.length === 4, `expected 4 nodes, got ${content.nodes.length}`);

  const child = navigateKey(ws, "child", { stock: { ok: false, paths: null } });
  ok(child.ok, "child navigate ok");
  ok(child.outbound_count === 3, `child outbound 3, got ${child.outbound_count}`);
  ok(child.inbound_count === 1, `child inbound 1 (from root), got ${child.inbound_count}`);
  ok(child.inbound.some((i) => i.key === "root"), "child inbound includes root");
  ok(child.missing_targets.includes("missing-one"), "missing-one flagged");
  ok(!child.parent.missing, "child has parent");
  ok(child.parent.keys.includes("root"), "child parent is root");
  ok(!child.orphan, "child is not orphan");

  const root = navigateKey(ws, "root", { stock: { ok: false, paths: null } });
  ok(root.ok, "root navigate ok");
  ok(root.parent.missing, "root has no 上层/父节点");
  // orphan also cites root as parent → inbound should be child, sibling, orphan = 3
  ok(root.inbound_count === 3, `root inbound 3, got ${root.inbound_count}`);
  ok(root.outbound.some((o) => o.key === "child"), "root outbound child");

  const orphan = navigateKey(ws, "orphan", { stock: { ok: false, paths: null } });
  ok(orphan.ok && orphan.orphan, "orphan flagged when inbound=0");

  const orph = navigateOrphans(ws);
  ok(orph.ok, "orphans ok");
  ok(orph.orphan_count === 1, `orphan_count 1, got ${orph.orphan_count}`);
  ok(orph.orphans[0].key === "orphan", "orphan key");

  const missing = navigateKey(ws, "no-such-key", { stock: { ok: false, paths: null } });
  ok(!missing.ok, "missing key fails");

  // Live corpus smoke: session-model has inbound+outbound
  section("kb_nav live corpus smoke");
  const live = navigateKey(repo, "session-model");
  ok(live.ok, "session-model exists");
  ok(live.outbound_count > 0, "session-model has outbound");
  ok(live.inbound_count > 0, "session-model has inbound");
  ok(Array.isArray(live.missing_targets), "missing_targets array");
}

function runScaffoldTests() {
  section("kb_scaffold");
  const domains = ["harness-core", "knowledge", "client-ux"];
  const built = buildScaffold({
    key: "wp013-demo",
    domain: "harness-core",
    parent: "session",
    dir: "knowledge/contracts",
    head: "deadbeef",
    domains,
    repo,
  });
  ok(built.ok, `scaffold build: ${built.error || "ok"}`);
  ok(built.relPath === "knowledge/contracts/wp013-demo.md", `relPath ${built.relPath}`);
  ok(built.markdown.includes("node : wp013-demo"), "decl key");
  ok(built.markdown.includes("status : pending"), "pending");
  ok(built.markdown.includes("domain : harness-core"), "domain");
  ok(built.markdown.includes('上层：[@ key="session"]'), "parent cite");
  ok(built.markdown.includes("基线：deadbeef"), "baseline");

  const parsed = parseKnowledgeMarkdown("contracts/wp013-demo.md", built.markdown);
  ok(parsed.key === "wp013-demo", `parsed key ${parsed.key}`);
  ok(parsed.status === "pending", `parsed status ${parsed.status}`);
  ok(parsed.refs.includes("session"), `parsed refs ${JSON.stringify(parsed.refs)}`);
  const dom = (parsed.extras.find((e) => /^domain\s*:/.test(e)) || "").replace(/^domain\s*:\s*/, "");
  ok(dom === "harness-core", `parsed domain ${dom}`);
  const pi = parentInfo(parsed.body, parsed.bodyStartLine);
  ok(pi.parentKeys.includes("session"), "parentInfo sees session");
  ok(pi.baseline && pi.baseline.hash === "deadbeef", "parentInfo baseline");

  const badDom = buildScaffold({ key: "x", domain: "team-ops", domains, repo });
  ok(!badDom.ok, "team-ops domain rejected");

  const badKey = buildScaffold({ key: "!!!", domain: "harness-core", domains, repo });
  ok(!badKey.ok, "bad key rejected");

  // write to temp only
  const tmpWrite = path.join(tmpRoot, "scaffold-write");
  fs.rmSync(tmpWrite, { recursive: true, force: true });
  fs.mkdirSync(tmpWrite, { recursive: true });
  const built2 = buildScaffold({
    key: "tmp-node",
    domain: "knowledge",
    parent: "knowledge",
    dir: "knowledge",
    head: "abc1234",
    domains,
    repo: tmpWrite,
  });
  // buildScaffold joins opts.repo + relPath; ensure knowledge dir conceptually under tmp
  // Override absPath manually for isolated write:
  const abs = path.join(tmpWrite, built2.relPath);
  const isolated = { ...built2, absPath: abs, repo: tmpWrite };
  const w1 = writeScaffold(isolated);
  ok(w1.ok && w1.written, `write ok: ${w1.error || "ok"}`);
  ok(fs.existsSync(abs), "file created");
  const w2 = writeScaffold(isolated);
  ok(!w2.ok && /already exists/.test(w2.error || ""), "refuse overwrite");
}

try {
  fs.rmSync(tmpRoot, { recursive: true, force: true });
  fs.mkdirSync(tmpRoot, { recursive: true });
  if (!scaffoldOnly) runNavTests();
  runScaffoldTests();
} finally {
  fs.rmSync(tmpRoot, { recursive: true, force: true });
}

const summary = { ok: fails.length === 0, failed: fails.length, failures: fails };
if (json) {
  console.log(JSON.stringify(summary, null, 2));
} else {
  if (fails.length) {
    console.log(`FAIL ${fails.length}`);
    for (const f of fails) console.log(`  - ${f}`);
  } else {
    console.log("PASS kb_nav/kb_scaffold selftest");
  }
  console.log(JSON.stringify({ ok: summary.ok, failed: summary.failed }));
}
process.exit(fails.length === 0 ? 0 : 1);

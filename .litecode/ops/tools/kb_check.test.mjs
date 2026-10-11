// Conformance: run every case of tests/fixtures/knowledge_refs/cases.json through
// the Node port (kb_parse.mjs) the same way src/knowledge/spec_tests.rs runs Rust.
// Usage: node .litecode/ops/tools/kb_check.test.mjs [--json]
// Temp workspaces live under .litecode/ops/.kb_check_tmp/ (removed afterwards);
// GIT_CEILING_DIRECTORIES keeps git from discovering the enclosing litecode repo.
import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { extractCites, normPath, refKey, formatLineSpan, computeOnce } from "./kb_parse.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const ops = path.join(here, "..");
const repo = path.resolve(ops, "..", "..");

const canon = (v) => JSON.stringify(v, (k, x) => (x && typeof x === "object" && !Array.isArray(x) ? Object.fromEntries(Object.entries(x).sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))) : x));

function extractActual(text) {
  return extractCites(text).map((c) => {
    const o = { kind: c.kind };
    if (c.kind === "node") o.key = c.key;
    else {
      o.path = normPath(c);
      if (c.kind === "symbol") o.symbol = c.symbol;
      if (c.kind !== "file") { o.lines = c.lines ? formatLineSpan(c.lines) : null; if (c.bad_lines !== null) o.bad_lines = c.bad_lines; }
    }
    o.ref_key = refKey(c);
    return o;
  });
}

function writeTree(root, files) {
  for (const [rel, text] of Object.entries(files || {})) {
    const p = path.join(root, rel);
    fs.mkdirSync(path.dirname(p), { recursive: true });
    fs.writeFileSync(p, text);
  }
}
function gitEnvFor(tmpRoot) {
  return { ...process.env, GIT_CEILING_DIRECTORIES: tmpRoot, GIT_CONFIG_NOSYSTEM: "1", GIT_CONFIG_GLOBAL: "/dev/null", GIT_AUTHOR_NAME: "t", GIT_AUTHOR_EMAIL: "t@l", GIT_COMMITTER_NAME: "t", GIT_COMMITTER_EMAIL: "t@l" };
}
function git(ws, env, args) {
  const r = spawnSync("git", args, { cwd: ws, env, encoding: "utf8" });
  if (r.status !== 0) throw new Error(`git ${args.join(" ")}: ${r.stderr}`);
}
function workspace(tmpRoot, c, idx) {
  const ws = path.join(tmpRoot, `case-${idx}`);
  fs.mkdirSync(ws, { recursive: true });
  const env = gitEnvFor(tmpRoot);
  if (c.head) {
    git(ws, env, ["-c", "init.defaultBranch=main", "init", "-q"]);
    for (const [k, v] of [["user.email", "test@litecode.local"], ["user.name", "Litecode Test"], ["commit.gpgsign", "false"], ["core.autocrlf", "false"]]) git(ws, env, ["config", k, v]);
    writeTree(ws, c.head);
    git(ws, env, ["add", "-A"]);
    git(ws, env, ["commit", "-q", "-m", "head"]);
  }
  writeTree(ws, c.files);
  for (const rel of c.delete || []) fs.rmSync(path.join(ws, rel));
  for (const rel of c.dirs || []) fs.mkdirSync(path.join(ws, rel), { recursive: true });
  for (const n of c.nodes || []) {
    const rel = n.path || `${n.key}.md`;
    const p = path.join(ws, "knowledge", rel);
    fs.mkdirSync(path.dirname(p), { recursive: true });
    fs.writeFileSync(p, `\`\`\`node\nnode : ${n.key}\nstatus : ${n.status || "enabled"}\nsummary : ${n.key}\n\`\`\`\n${n.body || ""}\n`);
  }
  return { ws, env };
}
const tripleSet = (arr) => new Set(arr.map((t) => JSON.stringify(t)));
const setEq = (a, b) => a.size === b.size && [...a].every((x) => b.has(x));

export function runConformance() {
  const table = JSON.parse(fs.readFileSync(path.join(repo, "tests/fixtures/knowledge_refs/cases.json"), "utf8"));
  const tmpRoot = path.join(ops, ".kb_check_tmp", `run-${process.pid}`);
  fs.mkdirSync(tmpRoot, { recursive: true });
  const results = [];
  try {
    for (const c of table.extract) {
      if (c.divergence) { results.push({ section: "extract", id: c.id, ok: true, skipped: "divergence" }); continue; }
      const got = extractActual(c.text);
      const ok = canon(got) === canon(c.refs);
      results.push({ section: "extract", id: c.id, ok, ...(ok ? {} : { wanted: c.refs, got }) });
    }
    table.resolve.forEach((c, idx) => {
      if (c.divergence) { results.push({ section: "resolve", id: c.id, ok: true, skipped: "divergence" }); return; }
      const { ws, env } = workspace(tmpRoot, c, idx);
      const content = computeOnce(ws, { gitEnv: env });
      const got = [];
      let consistent = true;
      for (const n of content.nodes) {
        for (const i of n.issues) got.push([n.path, i.code, i.ref_key ?? i.ref ?? ""]);
        for (const i of n.issues) if (i.ref_key && !(n.refs.find((r) => r.ref_key === i.ref_key)?.status === i.code)) consistent = false;
        if (new Set(n.refs.map((r) => r.ref_key)).size !== n.refs.length) consistent = false;
      }
      const wanted = (c.expect || []).map((e) => [e.node, e.status, e.ref_key ?? e.ref ?? ""]);
      const ok = consistent && setEq(tripleSet(got), tripleSet(wanted));
      results.push({ section: "resolve", id: c.id, ok, ...(ok ? {} : { wanted, got, consistent }) });
    });
    table.layout.forEach((c, idx) => {
      const ws = path.join(tmpRoot, `layout-${idx}`);
      writeTree(path.join(ws, "knowledge"), c.knowledge);
      for (const rel of c.dirs || []) fs.mkdirSync(path.join(ws, "knowledge", rel), { recursive: true });
      const content = computeOnce(ws, { gitEnv: gitEnvFor(tmpRoot) });
      const sorted = (arr) => arr.map((x) => canon(x)).sort();
      const got = [sorted(content.folders), sorted(content.unknown.map((u) => ({ path: u.path, folder_id: u.folder_id }))), sorted(content.nodes.map((n) => n.path))];
      const wanted = [sorted(c.folders), sorted(c.unknown), sorted(c.nodes)];
      const ok = canon(got) === canon(wanted);
      results.push({ section: "layout", id: c.id, ok, ...(ok ? {} : { wanted, got }) });
    });
  } finally {
    fs.rmSync(tmpRoot, { recursive: true, force: true });
    try { fs.rmdirSync(path.join(ops, ".kb_check_tmp")); } catch {}
  }
  const pass = results.filter((r) => r.ok).length;
  return { pass, total: results.length, by_section: Object.fromEntries(["extract", "resolve", "layout"].map((s) => [s, `${results.filter((r) => r.section === s && r.ok).length}/${results.filter((r) => r.section === s).length}`])), failures: results.filter((r) => !r.ok) };
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const r = runConformance();
  if (process.argv.includes("--json")) console.log(JSON.stringify(r, null, 2));
  else {
    console.log(`cases.json conformance: ${r.pass}/${r.total} (extract ${r.by_section.extract}, resolve ${r.by_section.resolve}, layout ${r.by_section.layout})`);
    for (const f of r.failures) console.log("FAIL", f.section, f.id, JSON.stringify({ wanted: f.wanted, got: f.got, consistent: f.consistent }));
  }
  process.exit(r.pass === r.total ? 0 : 1);
}

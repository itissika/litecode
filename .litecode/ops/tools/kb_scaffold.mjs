// ops.kb_scaffold — new knowledge node scaffolding (tools_version >= 12).
// Emits a declaration block + 上层/父节点 + 基线 + tag placeholders.
// Domain must be a product domain in board.json (not team-ops).
// Default: dry-run to stdout. --write creates the file only if missing.
//
// CLI: node .litecode/ops/tools/kb_scaffold.mjs --key <key> --domain <id>
//        [--parent <key>] [--dir knowledge/...] [--tag 上层|父节点]
//        [--write] [--json] [--wp WP-xxx] [--selftest]
// Exit: 0 ok · 1 refused (exists / bad domain) · 2 usage.
import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { loadRegistry, printVersionHeader, opsRoot, recordTrace } from "./_lib.mjs";
import { parseKnowledgeMarkdown, isKnowledgeKey, normalizeKey } from "./kb_parse.mjs";
import { ALLOWED_TAGS } from "./kb_check.mjs";

export const TOOL_ID = "ops.kb_scaffold";
const here = fileURLToPath(import.meta.url);
const OPS = opsRoot(import.meta.url);
export const REPO = path.resolve(OPS, "..", "..");

const PARENT_CHOICE = new Set(["上层", "父节点"]);

function productDomains() {
  const b = JSON.parse(fs.readFileSync(path.join(OPS, "board.json"), "utf8"));
  return b.domains.map((d) => d.id).filter((id) => id !== "team-ops");
}

function shortHead(cwd = REPO) {
  const r = spawnSync("git", ["rev-parse", "--short", "HEAD"], { cwd, encoding: "utf8" });
  if (r.status !== 0) return "0000000";
  return (r.stdout || "").trim().slice(0, 7) || "0000000";
}

/**
 * Build markdown template for a new pending node.
 * @returns {{ markdown, relPath, absPath, meta }}
 */
export function buildScaffold(opts) {
  const key = normalizeKey(opts.key || "");
  const domain = (opts.domain || "").trim();
  const parent = opts.parent ? normalizeKey(opts.parent) : null;
  const tag = PARENT_CHOICE.has(opts.tag) ? opts.tag : "上层";
  const domains = opts.domains || productDomains();
  const head = opts.head || shortHead(opts.repo || REPO);
  const dirArg = (opts.dir || "knowledge").replace(/\\/g, "/").replace(/\/+$/, "");

  if (!key || !isKnowledgeKey(key)) {
    return { ok: false, error: `invalid key "${opts.key}"` };
  }
  if (!domain) return { ok: false, error: "missing --domain" };
  if (!domains.includes(domain)) {
    return { ok: false, error: `domain "${domain}" 不在 board.json 产品域里（可选：${domains.join(", ")}）`, domains };
  }
  if (parent && !isKnowledgeKey(parent)) {
    return { ok: false, error: `invalid --parent "${opts.parent}"` };
  }

  // Resolve write path: --dir is under repo; if it already ends with .md use as file, else key.md inside dir.
  let relPath;
  if (dirArg.endsWith(".md")) {
    relPath = dirArg.replace(/^\.\//, "");
  } else {
    const base = dirArg.startsWith("knowledge/") || dirArg === "knowledge"
      ? dirArg
      : dirArg.startsWith("knowledge")
        ? dirArg
        : `knowledge/${dirArg.replace(/^knowledge\/?/, "")}`;
    relPath = `${base.replace(/\/+$/, "")}/${key}.md`;
  }
  // normalize double knowledge/
  relPath = relPath.replace(/\\/g, "/").replace(/\/+/g, "/");
  if (!relPath.startsWith("knowledge/")) {
    return { ok: false, error: `--dir must stay under knowledge/ (got ${relPath})` };
  }

  const parentLine = parent
    ? `${tag}：[@ key="${parent}"]。`
    : `${tag}：[@ key="<parent-key>"]。  <!-- TODO: 换成真实父节点 -->`;

  const tagHelp = ALLOWED_TAGS.filter((t) => !PARENT_CHOICE.has(t)).join(" / ");
  const markdown = [
    "```node",
    `node : ${key}`,
    "status : pending",
    `summary : `,
    `domain : ${domain}`,
    "```",
    "",
    `**主张：** （一句话）`,
    "",
    parentLine,
    `基线：${head}`,
    "",
    `<!-- 关系标签（选用）：${tagHelp} -->`,
    `<!-- 例：依据：[@ file="src/…"] 或 [@ file="src/…" symbol="…"] -->`,
    `<!-- 例：相关：[@ key="…"] -->`,
    "",
  ].join("\n");

  const absPath = path.join(opts.repo || REPO, relPath);
  return {
    ok: true,
    key,
    domain,
    parent,
    tag,
    head,
    relPath,
    absPath,
    markdown,
    domains,
  };
}

export function writeScaffold(built, { force = false } = {}) {
  if (!built.ok) return built;
  if (fs.existsSync(built.absPath) && !force) {
    return { ...built, ok: false, error: `file already exists: ${built.relPath}`, written: false };
  }
  fs.mkdirSync(path.dirname(built.absPath), { recursive: true });
  fs.writeFileSync(built.absPath, built.markdown, "utf8");
  return { ...built, written: true };
}

function main() {
  const { reg } = loadRegistry(import.meta.url);
  const args = process.argv.slice(2);
  const arg = (n) => { const i = args.indexOf(n); return i >= 0 ? args[i + 1] : null; };
  const json = args.includes("--json");
  const doWrite = args.includes("--write");
  printVersionHeader(reg, TOOL_ID);
  const known = new Set(["--key", "--domain", "--parent", "--dir", "--tag", "--write", "--json", "--wp", "--selftest", "--head"]);
  for (let i = 0; i < args.length; i++) {
    if (!known.has(args[i])) {
      console.error(`unknown argument ${args[i]}\nusage: kb_scaffold.mjs --key <key> --domain <id> [--parent <key>] [--dir knowledge/...] [--tag 上层|父节点] [--write] [--json] [--wp WP-xxx]`);
      process.exit(2);
    }
    if (["--key", "--domain", "--parent", "--dir", "--tag", "--wp", "--head"].includes(args[i])) {
      if (!args[i + 1] || args[i + 1].startsWith("--")) { console.error(`${args[i]} needs a value`); process.exit(2); }
      i++;
    }
  }
  const wp = arg("--wp");
  if (args.includes("--selftest")) {
    const r = spawnSync(process.execPath, [path.join(path.dirname(here), "kb_nav.test.mjs"), "--scaffold-only", ...(json ? ["--json"] : [])], { encoding: "utf8" });
    process.stdout.write(r.stdout || "");
    process.stderr.write(r.stderr || "");
    recordTrace(OPS, wp, TOOL_ID, reg.tools_version);
    process.exit(r.status ?? 1);
  }

  const built = buildScaffold({
    key: arg("--key"),
    domain: arg("--domain"),
    parent: arg("--parent"),
    dir: arg("--dir"),
    tag: arg("--tag"),
    head: arg("--head"),
  });
  if (!built.ok) {
    recordTrace(OPS, wp, TOOL_ID, reg.tools_version);
    if (json) console.log(JSON.stringify({ ok: false, tools_version: reg.tools_version, error: built.error, domains: built.domains }, null, 2));
    else console.error(`kb_scaffold: ${built.error}`);
    process.exit(built.error && built.error.includes("invalid") ? 2 : 1);
  }

  let result = { ...built, written: false };
  if (doWrite) {
    result = writeScaffold(built);
    if (!result.ok) {
      recordTrace(OPS, wp, TOOL_ID, reg.tools_version);
      if (json) console.log(JSON.stringify({ ok: false, tools_version: reg.tools_version, error: result.error, relPath: result.relPath }, null, 2));
      else console.error(`kb_scaffold: ${result.error}`);
      process.exit(1);
    }
  }

  // Sanity: parse the template
  const parsed = parseKnowledgeMarkdown(result.relPath.replace(/^knowledge\//, ""), result.markdown);
  recordTrace(OPS, wp, TOOL_ID, reg.tools_version);

  const out = {
    ok: true,
    tools_version: reg.tools_version,
    key: result.key,
    domain: result.domain,
    parent: result.parent,
    head: result.head,
    relPath: result.relPath,
    written: !!result.written,
    parsed: { key: parsed.key, status: parsed.status, domain: (parsed.extras.find((e) => /^domain\s*:/.test(e)) || "").replace(/^domain\s*:\s*/, ""), refs: parsed.refs },
  };

  if (json) {
    console.log(JSON.stringify({ ...out, markdown: result.markdown }, null, 2));
  } else {
    console.log(`kb_scaffold: key=${result.key} domain=${result.domain} path=${result.relPath} written=${!!result.written} head=${result.head}`);
    if (!result.written) {
      console.log("---");
      process.stdout.write(result.markdown);
      if (!result.markdown.endsWith("\n")) console.log();
      console.log("---");
    } else {
      console.log(`wrote ${result.relPath}`);
    }
    console.log(JSON.stringify(out));
  }
  process.exit(0);
}

if (process.argv[1] && path.resolve(process.argv[1]) === here) main();

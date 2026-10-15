// ops.toolchain_check (v2 tools_version 20: env -i row = source cargo_env.sh + ops .toolchain cargo == rust-toolchain.toml pin; v1, tools_version 15, WP-032) — is cargo/rustup usable in every bot shell flavour?
// Runs `cargo -V` and `rustup show active-toolchain` in: (1) inherited env (the bot's real Shell env),
// (2) non-login `bash -c`, (3) `bash -lc`, (4) minimal `env -i`. Any failure → exit 1.
// Prints which-paths, first PATH entries and whether RUSTUP_HOME/CARGO_HOME are set (values of other env not printed).
// Usage: node .litecode/ops/tools/toolchain_check.mjs [--json] [--wp WP-x]
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { loadRegistry, printVersionHeader, opsRoot, recordTrace } from "./_lib.mjs";
const TOOL = "ops.toolchain_check";
const { reg } = loadRegistry(import.meta.url);
printVersionHeader(reg, TOOL);
const ops = opsRoot(import.meta.url);
const args = process.argv.slice(2);
const probe = 'echo "which_cargo=$(command -v cargo)"; echo "path_head=$(echo "$PATH" | cut -d: -f1-3)"; echo "rustup_home_set=${RUSTUP_HOME:+yes}"; cargo -V 2>&1 | head -1; rustup show active-toolchain 2>&1 | head -1';
// v20 (WP-171): repo root + pinned channel (rust-toolchain.toml). The minimal env -i row sources cargo_env.sh first and must
// resolve the repo's .litecode/ops/.toolchain cargo at exactly the pinned version (system cargo e.g. 1.85 → FAIL).
const root = path.resolve(ops, "..", "..");
const pin = (() => { try { return (fs.readFileSync(path.join(root, "rust-toolchain.toml"), "utf8").match(/^\s*channel\s*=\s*"([^"]+)"/m) || [])[1] || null; } catch { return null; } })();
const opsCargo = path.join(root, ".litecode", "ops", ".toolchain", "cargo", "bin", "cargo");
const sq = (x) => "'" + x.replace(/'/g, "'\\''") + "'";
const flavours = [
  { name: "inherited", cmd: ["sh", ["-c", probe]], env: process.env },
  { name: "bash -c (non-login)", cmd: ["bash", ["-c", probe]], env: process.env },
  { name: "bash -lc (login)", cmd: ["bash", ["-lc", probe]], env: process.env },
  { name: "env -i (minimal)", cmd: ["env", ["-i", "PATH=/usr/local/bin:/usr/bin:/bin", `HOME=${process.env.HOME || "/home/box"}`, "bash", "-c", `source ${sq(path.join(root, ".litecode", "ops", "tools", "cargo_env.sh"))} && ${probe}`]], env: {}, cwd: root, strict: true },
];
const out = [];
for (const f of flavours) {
  const r = spawnSync(f.cmd[0], f.cmd[1], { encoding: "utf8", env: f.env, cwd: f.cwd || "/tmp", timeout: 30000 });
  const lines = (r.stdout || "").trim().split("\n");
  const kv = Object.fromEntries(lines.filter((l) => /^[a-z_]+=/.test(l)).map((l) => [l.split("=")[0], l.slice(l.indexOf("=") + 1)]));
  const rest = lines.filter((l) => !/^[a-z_]+=/.test(l));
  const cargo = rest[0] || "", active = rest[1] || "";
  let ok = /^cargo \d/.test(cargo) && !/error|no default|not found/i.test(active) && active.length > 0;
  let reason = ok ? null : "cargo/rustup unusable";
  if (f.strict && ok) {
    if (kv.which_cargo !== opsCargo) { ok = false; reason = `resolved ${kv.which_cargo} != ops toolchain ${opsCargo}`; }
    else if (!pin) { ok = false; reason = "rust-toolchain.toml channel missing"; }
    else if (!cargo.startsWith(`cargo ${pin} `) && cargo !== `cargo ${pin}`) { ok = false; reason = `version ${cargo} != pin ${pin}`; }
  }
  out.push({ flavour: f.name, ok, reason, which_cargo: kv.which_cargo || null, path_head: kv.path_head || null, rustup_home_set: kv.rustup_home_set === "yes", cargo, active_toolchain: active, ...(f.strict ? { expect_cargo: opsCargo, pin } : {}) });
}
const ok = out.every((o) => o.ok);
recordTrace(ops, args.includes("--wp") ? args[args.indexOf("--wp") + 1] : null, TOOL, reg.tools_version);
if (args.includes("--json")) console.log(JSON.stringify({ ok, results: out }, null, 2));
else { for (const o of out) console.log(`${o.ok ? "ok  " : "FAIL"} ${o.flavour.padEnd(20)} cargo=${o.which_cargo} | ${o.cargo} | ${o.active_toolchain} | RUSTUP_HOME set=${o.rustup_home_set} | PATH head=${o.path_head}${o.reason ? " | " + o.reason : ""}`); console.log(JSON.stringify({ ok })); }
process.exit(ok ? 0 : 1);

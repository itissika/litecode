// ops.toolchain_check (v1, tools_version 15, WP-032) — is cargo/rustup usable in every bot shell flavour?
// Runs `cargo -V` and `rustup show active-toolchain` in: (1) inherited env (the bot's real Shell env),
// (2) non-login `bash -c`, (3) `bash -lc`, (4) minimal `env -i`. Any failure → exit 1.
// Prints which-paths, first PATH entries and whether RUSTUP_HOME/CARGO_HOME are set (values of other env not printed).
// Usage: node .litecode/ops/tools/toolchain_check.mjs [--json] [--wp WP-x]
import { spawnSync } from "node:child_process";
import { loadRegistry, printVersionHeader, opsRoot, recordTrace } from "./_lib.mjs";
const TOOL = "ops.toolchain_check";
const { reg } = loadRegistry(import.meta.url);
printVersionHeader(reg, TOOL);
const ops = opsRoot(import.meta.url);
const args = process.argv.slice(2);
const probe = 'echo "which_cargo=$(command -v cargo)"; echo "path_head=$(echo "$PATH" | cut -d: -f1-3)"; echo "rustup_home_set=${RUSTUP_HOME:+yes}"; cargo -V 2>&1 | head -1; rustup show active-toolchain 2>&1 | head -1';
const flavours = [
  { name: "inherited", cmd: ["sh", ["-c", probe]], env: process.env },
  { name: "bash -c (non-login)", cmd: ["bash", ["-c", probe]], env: process.env },
  { name: "bash -lc (login)", cmd: ["bash", ["-lc", probe]], env: process.env },
  { name: "env -i (minimal)", cmd: ["env", ["-i", "PATH=/usr/local/bin:/usr/bin:/bin", `HOME=${process.env.HOME || "/home/box"}`, "sh", "-c", probe]], env: {} },
];
const out = [];
for (const f of flavours) {
  const r = spawnSync(f.cmd[0], f.cmd[1], { encoding: "utf8", env: f.env, cwd: "/tmp", timeout: 30000 });
  const lines = (r.stdout || "").trim().split("\n");
  const kv = Object.fromEntries(lines.filter((l) => /^[a-z_]+=/.test(l)).map((l) => [l.split("=")[0], l.slice(l.indexOf("=") + 1)]));
  const rest = lines.filter((l) => !/^[a-z_]+=/.test(l));
  const cargo = rest[0] || "", active = rest[1] || "";
  const ok = /^cargo \d/.test(cargo) && !/error|no default/i.test(active) && active.length > 0;
  out.push({ flavour: f.name, ok, which_cargo: kv.which_cargo || null, path_head: kv.path_head || null, rustup_home_set: kv.rustup_home_set === "yes", cargo, active_toolchain: active });
}
const ok = out.every((o) => o.ok);
recordTrace(ops, args.includes("--wp") ? args[args.indexOf("--wp") + 1] : null, TOOL, reg.tools_version);
if (args.includes("--json")) console.log(JSON.stringify({ ok, results: out }, null, 2));
else { for (const o of out) console.log(`${o.ok ? "ok  " : "FAIL"} ${o.flavour.padEnd(20)} cargo=${o.which_cargo} | ${o.cargo} | ${o.active_toolchain} | RUSTUP_HOME set=${o.rustup_home_set} | PATH head=${o.path_head}`); console.log(JSON.stringify({ ok })); }
process.exit(ok ? 0 : 1);

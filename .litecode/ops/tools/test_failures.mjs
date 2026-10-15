// ops.test_failures (v1, tools_version 20, WP-171): turn test logs into a forwardable failure list:
//   "- <step>: <test name> @ <file>:<line>[ — <first message line>]"
// Usage: node test_failures.mjs <step>=<log> ...   (step: cargo_test | web_test | desktop_test | ops_tests)
import fs from "node:fs";
const strip = (s) => s.replace(/\x1b\[[0-9;]*m/g, "");
export function cargo(text) {
  const out = [], lines = strip(text).split("\n");
  // runtime failures: "---- name stdout ----" then "panicked at file:line:col:" + message
  for (let i = 0; i < lines.length; i++) {
    const m = lines[i].match(/^---- (.+?) stdout ----$/); if (!m) continue;
    let loc = null, msg = null;
    for (let j = i + 1; j < Math.min(lines.length, i + 60) && !/^---- /.test(lines[j]); j++) {
      const p = lines[j].match(/panicked at ([^:]+):(\d+)(?::\d+)?:/);
      if (p && !loc) { loc = `${p[1]}:${p[2]}`; msg = (lines[j + 1] || "").trim(); }
    }
    out.push({ name: m[1], loc: loc || "?", msg });
  }
  // failed names with no stdout block (e.g. aborted) still listed
  for (const l of lines) { const m = l.match(/^test (.+) \.\.\. FAILED$/); if (m && !out.some((o) => o.name === m[1])) out.push({ name: m[1], loc: "?", msg: null }); }
  // compile errors: "error[E..]: msg" then "--> file:line:col"
  for (let i = 0; i < lines.length; i++) {
    const e = lines[i].match(/^error(\[E\d+\])?: (.+)$/); if (!e || /could not compile|test failed|targets? failed/.test(e[2])) continue;
    const a = lines.slice(i + 1, i + 6).map((l) => l.match(/-->\s+([^:]+):(\d+)/)).find(Boolean);
    if (a) out.push({ name: `compile error${e[1] || ""}`, loc: `${a[1]}:${a[2]}`, msg: e[2] });
  }
  return out;
}
export function vitest(text) {
  const out = [], lines = strip(text).split("\n");
  for (let i = 0; i < lines.length; i++) {
    const m = lines[i].match(/^\s*FAIL\s+(\S+)\s+>\s+(.+?)\s*$/); if (!m) continue;
    let loc = null, msg = null;
    for (let j = i + 1; j < Math.min(lines.length, i + 40) && !/^\s*FAIL\s/.test(lines[j]); j++) {
      if (!msg && /Error|expected|assert/i.test(lines[j])) msg = lines[j].trim();
      const p = lines[j].match(/❯\s+(\S+?):(\d+):\d+/) || lines[j].match(/\b(\S+\.(?:test|spec)\.[cm]?[jt]sx?):(\d+):\d+/);
      if (p && !loc) loc = `${p[1]}:${p[2]}`;
    }
    out.push({ name: m[2], loc: loc || `${m[1]}:?`, msg });
  }
  if (!out.length) for (const l of lines) { const m = l.match(/^\s*FAIL\s+(\S+)/); if (m) out.push({ name: "(suite)", loc: `${m[1]}:?`, msg: l.trim() }); }
  return out;
}
export function nodeTest(text) { // TAP from node --test (also tsc errors for desktop "npm test")
  const out = [], lines = strip(text).split("\n");
  for (let i = 0; i < lines.length; i++) {
    const m = lines[i].match(/^(\s*)not ok \d+ - (.+)$/); if (!m) continue;
    let loc = null, msg = null;
    for (let j = i + 1; j < Math.min(lines.length, i + 40) && !/^\s*(not )?ok \d+ /.test(lines[j]); j++) {
      const l = lines[j].match(/location:\s*'(.+?):(\d+):\d+'/); if (l && !loc) loc = `${l[1]}:${l[2]}`;
      const e = lines[j].match(/^\s*error:\s*(.+)$/); if (e && !msg) msg = e[1].replace(/^['|>-]\s*/, "").replace(/'$/, "").trim() || (lines[j + 1] || "").trim();
    }
    if (/^\s*(not ok \d+ - )?(subtestsFailed|.*\d+ subtests? failed)/.test(msg || "") && !loc) continue;
    out.push({ name: m[2], loc: loc || "?", msg });
  }
  for (const l of lines) { const t = l.match(/^(\S+\.tsx?)\((\d+),\d+\): error (TS\d+): (.+)$/); if (t) out.push({ name: `tsc ${t[3]}`, loc: `${t[1]}:${t[2]}`, msg: t[4] }); }
  return out;
}
const parsers = { cargo_test: cargo, web_test: vitest, desktop_test: nodeTest, ops_tests: nodeTest };
if (import.meta.url === `file://${process.argv[1]}`) {
  let total = 0; const rows = [];
  for (const a of process.argv.slice(2)) {
    const [step, file] = a.split("="); let items = [];
    try { items = (parsers[step] || nodeTest)(fs.readFileSync(file, "utf8")); } catch (e) { rows.push(`- ${step}: (log unreadable: ${file})`); continue; }
    if (!items.length) rows.push(`- ${step}: failed, but no test-level failure parsed — see ${file}`);
    for (const it of items) rows.push(`- ${step}: ${it.name} @ ${it.loc}${it.msg ? " — " + it.msg.slice(0, 200) : ""}`);
    total += items.length;
  }
  console.log(`TEST FAILURES: ${total} item(s)`); console.log(rows.join("\n"));
}

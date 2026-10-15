import fs from "node:fs"; const d = process.argv[2];
const runs = fs.readdirSync(d).filter((f) => /^run-\d+\.json$/.test(f)).map((f) => JSON.parse(fs.readFileSync(`${d}/${f}`)));
const med = (a) => { a = a.filter((x) => x != null).sort((x, y) => x - y); return a.length ? a[(a.length - 1) >> 1] : null; };
const k = ["fcp_from_nav", "T0_from_nav", "T_int_from_T0", "T_int_from_nav"];
const out = Object.fromEntries(k.map((x) => [x, med(runs.map((r) => r[x]))]));
out.serve_to_T0_ms = med(runs.map((r) => r.serveStart.wall_ms ? r.T0_wall - r.serveStart.wall_ms : null));
out.n = runs.length; fs.writeFileSync(`${d}/median.json`, JSON.stringify(out, null, 2)); console.log(out);

// WP-157 probe: one run. Usage: node probe.mjs <url> <out.json> [--reuse-context-dir DIR]
// Records navigationStart, FCP, T0 (first /ws open or first HTTP to serve), every HTTP/WS
// request start/end relative to T0, WS frames, and T_int (composer editable+focused+key echoes).
import { chromium } from "playwright";
import fs from "node:fs";
const [url, out, flag, dir] = process.argv.slice(2);
const serveStart = JSON.parse(process.env.SERVE_START_JSON || "{}");
const persistent = flag === "--reuse-context-dir";
const ctx = persistent
  ? await chromium.launchPersistentContext(dir, { headless: true })
  : await (await chromium.launch({ headless: true })).newContext();
const page = await ctx.newPage();
const t = () => performance.timeOrigin + performance.now(); // wall ms, monotonic-backed
const reqs = []; const frames = [];
page.on("request", (r) => { if (/\/(api|ws|health)/.test(new URL(r.url()).pathname)) reqs.push({ url: r.url(), method: r.method(), start: t(), r }); });
page.on("requestfinished", (r) => { const e = reqs.find((x) => x.r === r); if (e) e.end = t(); });
page.on("requestfailed", (r) => { const e = reqs.find((x) => x.r === r); if (e) { e.end = t(); e.failed = r.failure()?.errorText; } });
page.on("websocket", (ws) => {
  const w = { url: ws.url(), start: t(), kind: "ws" }; reqs.push(w);
  const rec = (dir) => (f) => { let m = ""; try { const j = JSON.parse(f.payload); m = j.method ?? (j.id != null ? `reply#${j.id}` : ""); if (j.id != null && j.method) m += `#${j.id}`; } catch {} frames.push({ dir, at: t(), m, len: String(f.payload).length }); };
  ws.on("framesent", rec("out")); ws.on("framereceived", rec("in"));
  ws.on("close", () => { w.end = t(); });
});
const navWall = t();
await page.goto(url, { waitUntil: "commit" });
// T_int: composer (MentionEditor, aria label "Message the agent") editable, focused, key echoes.
const sel = '[aria-label="Message the agent"]';
let tInt = null;
const deadline = Date.now() + 120_000;
while (Date.now() < deadline) {
  const loc = page.locator(sel).first();
  if (await loc.count().catch(() => 0)) {
    try {
      await loc.focus({ timeout: 200 });
      await page.keyboard.type("x");
      const txt = await loc.innerText({ timeout: 200 }).catch(() => "");
      const val = await loc.inputValue({ timeout: 50 }).catch(() => "");
      if ((txt + val).includes("x")) { tInt = t(); await page.keyboard.press("Backspace"); break; }
    } catch {}
  }
  await page.waitForTimeout(16);
}
const perf = await page.evaluate(() => ({
  timeOrigin: performance.timeOrigin,
  fcp: performance.getEntriesByName("first-contentful-paint")[0]?.startTime ?? null,
  dcl: performance.getEntriesByType("navigation")[0]?.domContentLoadedEventEnd ?? null,
}));
const T0 = Math.min(...reqs.map((r) => r.start));
const rel = (x) => (x == null ? null : +(x - T0).toFixed(1));
const res = {
  url, serveStart, navWall, navigationStart: perf.timeOrigin,
  fcp_from_nav: perf.fcp, dcl_from_nav: perf.dcl,
  T0_wall: T0, T0_from_nav: +(T0 - perf.timeOrigin).toFixed(1),
  T_int_from_T0: rel(tInt), T_int_from_nav: tInt ? +(tInt - perf.timeOrigin).toFixed(1) : null,
  requests: reqs.map(({ r, ...e }) => ({ ...e, url: e.url.replace(/token=[^&]+/, "token=***"), start: rel(e.start), end: rel(e.end), dur: e.end ? +(e.end - e.start).toFixed(1) : null })),
  ws_frames: frames.map((f) => ({ ...f, at: rel(f.at) })),
};
fs.writeFileSync(out, JSON.stringify(res, null, 2));
console.log(out, "T0_from_nav", res.T0_from_nav, "T_int_from_T0", res.T_int_from_T0);
await ctx.close(); process.exit(tInt ? 0 : 2);

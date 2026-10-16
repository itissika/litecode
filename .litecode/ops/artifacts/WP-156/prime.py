#!/usr/bin/env python3
"""Build the C2 session index: start release serve on WS with a fixed HOME, wait until
.litecode/session-index/meta.json has building=false and job.json is not running, then SIGTERM.
RSS watchdog: sample every 1s; stop if MemAvailable < 4.5GiB. usage: prime.py WS HOME PORT"""
import json, os, signal, subprocess, sys, time
ws, home, port = sys.argv[1], sys.argv[2], sys.argv[3]
R = "/workspace/litecode"; BIN = os.environ.get("WP156_BIN", f"{R}/target/release/litecode")
os.makedirs(home, exist_ok=True)
env = dict(os.environ, HOME=home, LITECODE_LOG="info", LITECODE_WEB_DIST=f"{R}/.tmp/wp102/dist"); env.pop("LITECODE_TOKEN", None)
log = open(f"{R}/.tmp/wp156/logs/prime_{os.path.basename(ws)}.log", "w")
t0 = time.time()
p = subprocess.Popen([BIN, "--workspace", ws, "serve", "--bind", f"127.0.0.1:{port}", "--loopback-only"], env=env, stdout=log, stderr=subprocess.STDOUT, cwd=ws, start_new_session=True)
idx = f"{ws}/.litecode/session-index"
peak = 0; minavail = 1 << 60; status = "timeout"; samples = []
def memavail():
    for l in open("/proc/meminfo"):
        if l.startswith("MemAvailable"): return int(l.split()[1])
def rss_tree(pid):
    tot = 0
    for d in os.listdir("/proc"):
        if not d.isdigit(): continue
        try:
            st = open(f"/proc/{d}/stat").read().split(")")[-1].split()
            if os.getpgid(int(d)) == pid:
                tot += int([l for l in open(f"/proc/{d}/status") if l.startswith("VmRSS")][0].split()[1])
        except (OSError, IndexError, ProcessLookupError): pass
    return tot
while time.time() - t0 < 1800:
    time.sleep(1)
    if p.poll() is not None: status = f"exited {p.returncode}"; break
    r = rss_tree(p.pid); m = memavail(); peak = max(peak, r); minavail = min(minavail, m)
    if m < float(os.environ.get("WP156_MIN_GIB", "4.5")) * 1024 * 1024: status = "stopped_low_mem"; break
    try:
        meta = json.load(open(f"{idx}/meta.json")); job = json.load(open(f"{idx}/job.json")) if os.path.exists(f"{idx}/job.json") else {}
        if not meta.get("building") and meta.get("last_change_id", 0) > 0 and job.get("state", job.get("status")) not in ("running",):
            status = "done"; break
        if int(time.time() - t0) % 30 == 0: samples.append({"t": int(time.time() - t0), "rss_kb": r, "job": {"status": job.get("status"), "chunks": job.get("chunks")}})
    except (OSError, ValueError): pass
end = time.time()
p.send_signal(signal.SIGTERM)
try: p.wait(20)
except subprocess.TimeoutExpired: pass
try: os.killpg(p.pid, signal.SIGKILL)  # serve leaves code-search-worker orphaned on SIGTERM
except ProcessLookupError: pass
print(json.dumps({"ws": ws, "status": status, "secs": round(end - t0, 1), "end_local": time.strftime("%H:%M:%S", time.localtime(end)), "peak_rss_tree_kb": peak, "min_memavail_kb": minavail, "samples": samples[-5:]}))

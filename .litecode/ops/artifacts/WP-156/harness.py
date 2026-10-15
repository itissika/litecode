#!/usr/bin/env python3
"""WP-156 harness (rebuilt after disk rollback). usage: harness.py GROUP(empty|corpus) COLD(C1|C2) RUNS WARM_PER_RUN
env: WP156_DIR (work dir, default /workspace/wp156-recover/run; holds small_repo, corpus_small.db, subscribe_sid.txt, logs/)
     WP156_BIN (default /workspace/litecode/target/release/litecode)  WP156_WS (workspace override)
     WP156_FRESH_CMD (C1 reset command, e.g. "fresh_big_c1.sh A")  WP156_PATH (A|B)  WP156_TAG (log prefix)
     WP156_HOME ("fresh" = new empty HOME per run, else a fixed primed HOME path)  WP156_TOKEN  WP156_ENGINES (engines.json to copy)
     WP156_WEB_DIST (default /workspace/litecode/.tmp/wp102/dist)
Per run: 6GiB MemAvailable gate (waits up to 10 min), leftover-process check before/after, page-cache eviction with
mincore readings (mincore.py), spawn->READY from the LITECODE_READY stdout line, client first connect + WARM reconnects,
whole process group killed at the end (serve leaves code-search-worker orphaned on SIGTERM, ISSUE-061)."""
import glob, json, os, shutil, signal, subprocess, sys, threading, time
HERE = os.path.dirname(os.path.abspath(__file__))
R = "/workspace/litecode"
W = os.environ.get("WP156_DIR", f"{HERE}/run")
BIN = os.environ.get("WP156_BIN", f"{R}/target/release/litecode")
MODEL = glob.glob(f"{R}/models/ibm-granite/granite-embedding-97m-multilingual-r2/**/*", recursive=True)
TAG = os.environ.get("WP156_TAG", "")
group, cold, runs, warm = sys.argv[1], sys.argv[2], int(sys.argv[3]), int(sys.argv[4])
ws = os.environ.get("WP156_WS", f"{W}/ws_{group}")
port = 7600 + (0 if group == "empty" else 1)
os.makedirs(f"{W}/logs", exist_ok=True)

def fresh_ws():
    if os.environ.get("WP156_FRESH_CMD"):  # large repo: reset a seed copy instead of copying small_repo
        subprocess.run(os.environ["WP156_FRESH_CMD"], shell=True, check=True); return
    shutil.rmtree(ws, ignore_errors=True); shutil.copytree(f"{W}/small_repo", ws, symlinks=True)
    os.makedirs(f"{ws}/.litecode", exist_ok=True)
    if os.environ.get("WP156_ENGINES"):  # C1 default: no engines.json (retrieval/LSP off, no embed)
        shutil.copy(os.environ["WP156_ENGINES"], f"{ws}/.litecode/engines.json")
    if group == "corpus":
        shutil.copy(f"{W}/corpus_small.db", f"{ws}/.litecode/sessions.db")

def leftovers():
    out = subprocess.run(["pgrep", "-f", "target/release/litecode|target/debug/litecode"], capture_output=True, text=True).stdout.split()
    return [x for x in out if x != str(os.getpid())]

def files_to_evict(home):
    fs = [BIN, home] + [p for p in MODEL if os.path.isfile(p)] + [ws]
    if os.environ.get("WP156_WEB_DIST"): fs.append(os.environ["WP156_WEB_DIST"])
    return fs

def mincore(cmd, home):
    out = subprocess.run(["python3", f"{HERE}/mincore.py", cmd, *files_to_evict(home)], capture_output=True, text=True).stdout
    d = json.loads(out)
    return {"resident_pages": d["resident_pages"], "total_pages": d["total_pages"], "files": d["files"], "still_resident": len(d["still_resident"])}

def client(path, sid):
    cenv = dict(os.environ, TOKEN=os.environ.get("WP156_TOKEN", ""))
    out = subprocess.run(["node", f"{HERE}/ws_client.mjs", f"127.0.0.1:{port}", path, sid or "", "1500"],
                         capture_output=True, text=True, timeout=90, env=cenv).stdout
    return json.loads(out.strip().splitlines()[-1])

def rss(pid):
    try:
        st = open(f"/proc/{pid}/status").read()
        return {k: int(l.split()[1]) for l in st.splitlines() for k in ("VmHWM", "VmRSS") if l.startswith(k + ":")}
    except OSError: return {}

def memavail():
    return int([l for l in open("/proc/meminfo") if l.startswith("MemAvailable")][0].split()[1])

sid = open(f"{W}/subscribe_sid.txt").read().strip() if group == "corpus" else None
path = os.environ.get("WP156_PATH", "B" if group == "corpus" else "A")
results = []
for i in range(runs):
    if cold == "C1": fresh_ws()
    elif not os.path.isdir(ws): sys.exit("C2 needs a primed workspace")
    if leftovers(): sys.exit(f"leftover litecode processes before run: {leftovers()}")
    hm = os.environ.get("WP156_HOME", "fresh")
    run_home = f"{W}/homes/{TAG}{group}_{cold}_{i}" if hm == "fresh" else hm
    if hm == "fresh": shutil.rmtree(run_home, ignore_errors=True)
    os.makedirs(run_home, exist_ok=True)
    for _ in range(60):  # wait up to 10 min for the 6GiB gate
        m = memavail()
        if m >= 6 * 1024 * 1024: break
        time.sleep(10)
    if m < 6 * 1024 * 1024: sys.exit(f"MemAvailable {m} kB < 6GiB before run {i}")
    env = dict(os.environ, HOME=run_home, LITECODE_LOG="debug", LITECODE_WEB_DIST=os.environ.get("WP156_WEB_DIST", f"{R}/.tmp/wp102/dist"))
    env.pop("LITECODE_TOKEN", None)
    tok = os.environ.get("WP156_TOKEN")  # serve requires it; client sends ?token= and auth{token}
    if tok: env["LITECODE_TOKEN"] = tok
    mc = mincore("evict", run_home)  # last step before spawn: nothing reads these files in between
    log = open(f"{W}/logs/{TAG}{group}_{cold}_{i}.serve.log", "w")
    ts = time.time(); tsm = time.monotonic()
    p = subprocess.Popen([BIN, "--workspace", ws, "serve", "--bind", f"127.0.0.1:{port}", "--loopback-only"], env=env,
                         stdout=subprocess.PIPE, stderr=subprocess.STDOUT, cwd=ws, text=True, bufsize=1, start_new_session=True)
    ready_evt = threading.Event(); ready_at = [None]
    def pump():
        for line in p.stdout:
            if ready_at[0] is None and line.startswith("LITECODE_READY"):
                ready_at[0] = time.monotonic() - tsm; ready_evt.set()
            log.write(line)
        ready_evt.set()
    threading.Thread(target=pump, daemon=True).start()
    ready_evt.wait(120); ready = ready_at[0]
    run = {"group": group, "cold": cold, "run": i, "home": hm, "spawn_epoch": ts,
           "spawn_to_ready_ms": round(ready * 1000, 1) if ready else None, "mincore_after_evict": mc}
    if ready is None:
        os.killpg(p.pid, signal.SIGKILL); sys.exit(f"serve did not come up, see logs/{TAG}{group}_{cold}_{i}.serve.log")
    run["first"] = client(path, sid)
    run["warm"] = []
    for k in range(warm):
        time.sleep(0.5)
        run["warm"].append(client(path, sid))
    run["rss_kb"] = rss(p.pid)
    p.send_signal(signal.SIGTERM)
    try: p.wait(15)
    except subprocess.TimeoutExpired: pass
    try: os.killpg(p.pid, signal.SIGKILL)  # ISSUE-061: worker would survive SIGTERM
    except ProcessLookupError: pass
    time.sleep(0.5); run["leftovers_after"] = leftovers()
    if run["leftovers_after"]: subprocess.run(["pkill", "-9", "-f", "target/release/litecode"])
    results.append(run)
    print(json.dumps({"run": i, "ready": run["spawn_to_ready_ms"], "unlock": run["first"].get("unlock"), "rss": run["rss_kb"]}), flush=True)
with open(f"{W}/logs/{TAG}{group}_{cold}.json", "w") as f: json.dump(results, f)

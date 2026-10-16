import json, statistics as st, sys, glob, os
def med(xs):
    xs = [x for x in xs if x is not None]; return round(st.median(xs), 1) if xs else None
def keys(c):
    m = c["marks"]; ev = c["events"]
    def first(pred):
        for e in ev:
            if e.get("dir") == "in" and pred(e): return e
    auth = first(lambda e: e["m"] == "reply:auth")
    sub = first(lambda e: e["m"] == "reply:session/subscribe")
    snap_r = first(lambda e: e["m"] == "session/snapshot" and e.get("sid") == c.get("restored")) if c.get("restored") else None
    snap_any = first(lambda e: e["m"] == "session/snapshot")
    lst = first(lambda e: e["m"] == "reply:session/list")
    new = first(lambda e: e["m"] == "reply:session/new")
    return {"ws_open": m.get("ws_open"), "hello": m.get("server/hello"), "GET engines": m.get("GET /api/workspace/engines"), "GET settings": m.get("GET /api/settings"),
            "list_reply": lst and lst["t"], "list_n": lst and lst.get("sessions"), "new_reply": new and new["t"], "subscribe_reply(1st)": sub and sub["t"],
            "snapshot(restored)": snap_r and snap_r["t"], "snapshot(1st)": snap_any and snap_any["t"], "auth_reply": auth and auth["t"], "auth_err": auth and auth.get("err"),
            "unlock": c.get("unlock")}
out = {}
for f in sorted(glob.glob("logs/*_C*.json")):
    runs = json.load(open(f)); name = os.path.basename(f)[:-5]
    firsts = [keys(r["first"]) for r in runs]; warms = [keys(w) for r in runs for w in r["warm"]]
    agg = lambda rows: {k: med([r[k] for r in rows]) if k not in ("auth_err",) else sorted({r[k] for r in rows}, key=str) for k in rows[0]}
    out[name] = {"n": len(runs), "spawn_to_ready": [r["spawn_to_ready_ms"] for r in runs], "spawn_to_ready_med": med([r["spawn_to_ready_ms"] for r in runs]),
                 "first_med": agg(firsts), "warm_med": agg(warms), "first_unlock_raw": [f["unlock"] for f in firsts], "warm_unlock_raw": [w["unlock"] for w in warms],
                 "rss_hwm_mb": [round(r["rss_kb"].get("VmHWM", 0) / 1024) for r in runs],
                 "mincore": [r["mincore_after_evict"] for r in runs], "leftovers_after": [r.get("leftovers_after") for r in runs]}
json.dump(out, open("logs/summary.json", "w"), indent=1)
for k, v in out.items():
    print("==", k, "ready", v["spawn_to_ready_med"], v["spawn_to_ready"], "rss", v["rss_hwm_mb"])
    print("  first", v["first_med"]); print("  warm ", v["warm_med"]); print("  mincore", [ (m["resident_pages"], m["total_pages"]) for m in v["mincore"]], "left", v["leftovers_after"])

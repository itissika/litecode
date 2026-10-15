#!/usr/bin/env bash
# WP-165: measure serve startup → LITECODE_READY and first language-server ready.
#
# Matrix (4 cells × RUNS):
#   fix=before|after  ×  residual=none|orphan
#
# "before" needs a binary built from the pre-fix tree (sync sweep in run.rs).
# "after"  is the current tree (start_background). Pass distinct --bin-before /
# --bin-after, or only --bin for the cell you can build today.
#
# Does NOT invoke cargo. Skip if cargo/rustc busy or MemAvailable < 3GiB.
#
# Usage:
#   ./measure_ready_vs_lsp.sh \
#     --bin-after /path/to/litecode \
#     [--bin-before /path/to/litecode-before] \
#     [--workspace /path/to/ws] \
#     [--runs 3] \
#     [--out timings.jsonl]
#
# Env:
#   LITECODE_TOKEN   required by --require-auth (default: wp165-measure)
#   LITECODE_WEB_DIST  web stub (default: .tmp/wp154/webstub if present)
#   LITECODE_HOME / HOME  scratch homes under OUTDIR
set -euo pipefail

ROOT=/workspace/litecode
ART="$ROOT/.litecode/ops/artifacts/WP-165"
OUT="$ART/timings.jsonl"
RUNS=3
BIN_BEFORE=""
BIN_AFTER=""
WS=""
MIN_MEM_KB=$((3 * 1024 * 1024))  # 3 GiB

while [[ $# -gt 0 ]]; do
  case "$1" in
    --bin) BIN_AFTER="$2"; shift 2 ;;
    --bin-after) BIN_AFTER="$2"; shift 2 ;;
    --bin-before) BIN_BEFORE="$2"; shift 2 ;;
    --workspace) WS="$2"; shift 2 ;;
    --runs) RUNS="$2"; shift 2 ;;
    --out) OUT="$2"; shift 2 ;;
    -h|--help) sed -n '2,30p' "$0"; exit 0 ;;
    *) echo "unknown arg: $1" >&2; exit 2 ;;
  esac
done

if [[ -z "$BIN_AFTER" && -z "$BIN_BEFORE" ]]; then
  echo "need --bin / --bin-after and/or --bin-before" >&2
  exit 2
fi

busy() {
  for p in /proc/[0-9]*; do
    c=$(cat "$p/comm" 2>/dev/null || true)
    [[ "$c" == cargo || "$c" == rustc ]] && { echo "$c"; return 0; }
  done
  return 1
}

mem_avail_kb() { awk '/MemAvailable/{print $2}' /proc/meminfo; }

if b=$(busy); then
  echo "SKIP: $b running — do not measure while the toolchain is busy" >&2
  exit 2
fi
avail=$(mem_avail_kb)
if (( avail < MIN_MEM_KB )); then
  echo "SKIP: MemAvailable ${avail}kB < ${MIN_MEM_KB}kB" >&2
  exit 3
fi

TOKEN="${LITECODE_TOKEN:-wp165-measure}"
WEB_DIST="${LITECODE_WEB_DIST:-$ROOT/.tmp/wp154/webstub}"
WORKDIR=$(mktemp -d /tmp/wp165-measure.XXXXXX)
trap 'rm -rf "$WORKDIR"; [[ -n "${SERVE_PID:-}" ]] && kill -9 "$SERVE_PID" 2>/dev/null || true; pkill -9 -f "$WORKDIR/lsp/orphan-sleep" 2>/dev/null || true' EXIT

if [[ -z "$WS" ]]; then
  WS="$WORKDIR/ws"
  mkdir -p "$WS/src"
  # Minimal Rust file so a rust-analyzer (if installed under lsp/) has something to open.
  printf '%s\n' 'fn main() {}' >"$WS/src/main.rs"
  printf '%s\n' '[package]' 'name = "wp165_probe"' 'version = "0.0.0"' 'edition = "2021"' >"$WS/Cargo.toml"
fi

LSP_DIR="$WORKDIR/lsp"
mkdir -p "$LSP_DIR"
# Orphan bait: copy sleep into managed lsp dir; reparent to init (ppid==1).
ORPHAN_BIN="$LSP_DIR/orphan-sleep"
cp "$(command -v sleep)" "$ORPHAN_BIN"
chmod +x "$ORPHAN_BIN"

spawn_orphans() {
  local n=${1:-3}
  python3 - "$ORPHAN_BIN" "$n" <<'PY'
import os, sys, time
bin_path, n = sys.argv[1], int(sys.argv[2])
for _ in range(n):
    pid = os.fork()
    if pid == 0:
        # intermediate exits → grandchild reparented to init/subreaper
        if os.fork() == 0:
            os.setsid()
            os.execv(bin_path, [bin_path, "600"])
        os._exit(0)
    os.waitpid(pid, 0)
time.sleep(0.2)
PY
}

count_orphans() {
  local c=0
  for p in /proc/[0-9]*; do
    exe=$(readlink "$p/exe" 2>/dev/null || true)
    [[ "$exe" == "$ORPHAN_BIN" ]] || continue
    stat=$(cat "$p/stat" 2>/dev/null || true)
    rest=${stat#*)}
    ppid=$(awk '{print $2}' <<<"$rest")
    [[ "$ppid" == 1 ]] && c=$((c+1))
  done
  echo "$c"
}

# First LS ready = first descendant of serve whose /proc/pid/exe lives under LSP_DIR
# and is NOT our orphan-sleep bait. Recorded relative to T0 (serve spawn).
first_ls_ms() {
  local serve_pid=$1 t0_ns=$2 deadline_s=${3:-120}
  local end=$((SECONDS + deadline_s))
  while (( SECONDS < end )); do
    # walk descendants
    python3 - "$serve_pid" "$LSP_DIR" "$ORPHAN_BIN" "$t0_ns" <<'PY' && return 0
import os, sys, time
serve, lsp_dir, orphan, t0_ns = int(sys.argv[1]), sys.argv[2], sys.argv[3], int(sys.argv[4])
# build parent map
procs = []
for name in os.listdir("/proc"):
    if not name.isdigit():
        continue
    try:
        st = open(f"/proc/{name}/stat").read()
        rest = st[st.rfind(")")+1:].split()
        procs.append((int(name), int(rest[1])))
    except Exception:
        pass
children = {}
for pid, ppid in procs:
    children.setdefault(ppid, []).append(pid)
stack = [serve]
desc = set()
while stack:
    x = stack.pop()
    for c in children.get(x, []):
        if c not in desc:
            desc.add(c)
            stack.append(c)
for pid in desc:
    try:
        exe = os.readlink(f"/proc/{pid}/exe")
    except Exception:
        continue
    if exe.startswith(lsp_dir) and exe != orphan:
        now_ns = time.monotonic_ns()
        ms = (now_ns - t0_ns) / 1e6
        print(f"{ms:.3f}")
        sys.exit(0)
sys.exit(1)
PY
    sleep 0.05
  done
  echo ""
  return 1
}

wait_ready_ms() {
  local log=$1 t0_ns=$2 timeout_s=${3:-90}
  local end=$((SECONDS + timeout_s)) line="" ms=""
  while (( SECONDS < end )); do
    if line=$(grep -m1 '^LITECODE_READY ' "$log" 2>/dev/null || true); then
      # approximate: READY observed time (log mtime not precise); use now - t0
      python3 - "$t0_ns" <<'PY'
import sys, time
t0 = int(sys.argv[1])
print(f"{(time.monotonic_ns()-t0)/1e6:.3f}")
PY
      echo "$line"
      return 0
    fi
    sleep 0.01
  done
  return 1
}

trigger_lsp_best_effort() {
  # Best-effort: hit engines + lsp probe so deferred activation / first need can run.
  local base=$1
  curl -fsS -H "Authorization: Bearer $TOKEN" "$base/api/workspace/engines" -o /dev/null || true
  curl -fsS -H "Authorization: Bearer $TOKEN" "$base/api/workspace/lsp/probe" -o /dev/null || true
}

run_one() {
  local fix=$1 residual=$2 bin=$3 run_i=$4
  local home="$WORKDIR/home-$fix-$residual-$run_i"
  local log="$WORKDIR/serve-$fix-$residual-$run_i.log"
  mkdir -p "$home"
  export HOME="$home" LITECODE_HOME="$home" LITECODE_LSP_DIR="$LSP_DIR"
  export LITECODE_TOKEN="$TOKEN"
  [[ -d "$WEB_DIST" ]] && export LITECODE_WEB_DIST="$WEB_DIST"

  pkill -9 -f "$ORPHAN_BIN" 2>/dev/null || true
  local orphan_n=0
  if [[ "$residual" == orphan ]]; then
    spawn_orphans 3
    orphan_n=$(count_orphans)
  fi

  local t0_ns
  t0_ns=$(python3 -c 'import time; print(time.monotonic_ns())')
  "$bin" --workspace "$WS" serve --bind 127.0.0.1:0 --require-auth >"$log" 2>&1 &
  SERVE_PID=$!

  local ready_ms="" ready_line="" base=""
  if ready_out=$(wait_ready_ms "$log" "$t0_ns" 90); then
    ready_ms=$(head -1 <<<"$ready_out")
    ready_line=$(tail -1 <<<"$ready_out")
    base=${ready_line#LITECODE_READY }
    base=${base%/}
  else
    ready_ms=""
    echo "TIMEOUT ready fix=$fix residual=$residual run=$run_i" >&2
    kill -9 "$SERVE_PID" 2>/dev/null || true
    SERVE_PID=""
    printf '{"fix":"%s","residual":"%s","run":%s,"ready_ms":null,"first_ls_ms":null,"orphan_count":%s,"bin":"%s","error":"ready_timeout"}\n' \
      "$fix" "$residual" "$run_i" "$orphan_n" "$bin" | tee -a "$OUT"
    return 0
  fi

  trigger_lsp_best_effort "$base"
  local ls_ms
  ls_ms=$(first_ls_ms "$SERVE_PID" "$t0_ns" 90 || true)

  kill -9 "$SERVE_PID" 2>/dev/null || true
  wait "$SERVE_PID" 2>/dev/null || true
  SERVE_PID=""
  pkill -9 -f "$ORPHAN_BIN" 2>/dev/null || true

  python3 - "$fix" "$residual" "$run_i" "$ready_ms" "$ls_ms" "$orphan_n" "$bin" "$OUT" <<'PY'
import json, sys
fix, residual, run_i, ready, ls, orphan_n, bin_path, out = sys.argv[1:9]
rec = {
  "fix": fix,
  "residual": residual,
  "run": int(run_i),
  "ready_ms": float(ready) if ready not in ("", "None") else None,
  "first_ls_ms": float(ls) if ls not in ("", "None") else None,
  "orphan_count_at_start": int(orphan_n),
  "bin": bin_path,
}
open(out, "a").write(json.dumps(rec, ensure_ascii=False) + "\n")
print(json.dumps(rec, ensure_ascii=False))
PY
}

echo "# WP-165 measure → $OUT" >&2
: >"$OUT"

cells=()
[[ -n "$BIN_BEFORE" ]] && cells+=("before:$BIN_BEFORE")
[[ -n "$BIN_AFTER" ]] && cells+=("after:$BIN_AFTER")

for cell in "${cells[@]}"; do
  fix=${cell%%:*}
  bin=${cell#*:}
  [[ -x "$bin" ]] || { echo "not executable: $bin" >&2; exit 4; }
  for residual in none orphan; do
    for i in $(seq 1 "$RUNS"); do
      if b=$(busy); then echo "SKIP mid-run: $b" >&2; exit 2; fi
      echo "## fix=$fix residual=$residual run=$i bin=$bin" >&2
      run_one "$fix" "$residual" "$bin" "$i"
      sleep 1
    done
  done
done

echo "done → $OUT" >&2

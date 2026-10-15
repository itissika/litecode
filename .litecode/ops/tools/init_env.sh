#!/usr/bin/env bash
# ops.init_env (v20, WP-171): fresh Linux machine -> buildable/testable litecode + usable ops toolset. Idempotent.
# Usage: bash init_env.sh [--repo URL] [--branch B] [--dir DIR] [--skip-heavy]
# Memory guard: heavy steps start only if MemAvailable >= LC_MIN_MEM_GIB (default 2 start gate; kill default 1) and are killed (no retry, exit 4) if it drops
# below LC_KILL_MEM_GIB (default 1; start gate stays 2GiB). CARGO_BUILD_JOBS default 2. One-time exceptions are passed via env only, never by editing defaults.
# Install/build/validate/toolchain_check are fail-fast; test stage runs all 4 test steps (cargo test --no-fail-fast) then exits 6 with
# $LOG_DIR/test_failures.txt listing each failure as "step: test @ file:line — message" (ops.test_failures) if any failed.
# Logs: LC_INIT_LOG_DIR overrides; else after clone default to $DIR/.tmp/init-logs (survives HOME wipe when
# DIR is under /workspace). Early steps may write to /tmp/litecode-init-$$ then relocate. Per heavy step:
# start_memavail_kb + min_memavail_kb (=peak used vs start) + seconds -> heavy_stats.tsv.
# Rust goes into $LC_ROOT/.litecode/ops/.toolchain (same place cargo_env.sh points to).
set -euo pipefail
REPO=https://github.com/itissika/litecode.git; BRANCH=ops/team-framework; DIR="$HOME/litecode"; SKIP_HEAVY=0
while [ $# -gt 0 ]; do case $1 in --repo) REPO=$2; shift;; --branch) BRANCH=$2; shift;; --dir) DIR=$2; shift;; --skip-heavy) SKIP_HEAVY=1;; *) echo "unknown arg $1"; exit 2;; esac; shift; done
gib2kb(){ awk -v g="$1" 'BEGIN{printf "%d", g*1048576}'; }; MIN_KB=$(gib2kb "${LC_MIN_MEM_GIB:-2}"); KILL_KB=$(gib2kb "${LC_KILL_MEM_GIB:-1}"); NODE_MAJOR=${LC_NODE_MAJOR:-22}
if [ -n "${LC_INIT_LOG_DIR:-}" ]; then LOG_DIR="$LC_INIT_LOG_DIR"; LOG_RELOCATE=0
else LOG_DIR="/tmp/litecode-init-$$"; LOG_RELOCATE=1; fi
mkdir -p "$LOG_DIR"
step(){ echo "=== [$(date '+%F %T %Z')] $*" | tee -a "$LOG_DIR/init.log"; }
memavail(){ awk '/MemAvailable/{print $2}' /proc/meminfo; }
heavy(){ # heavy <name> <cmd...>: memory gate + watchdog + stats
  local name=$1; shift
  if [ "$SKIP_HEAVY" = 1 ]; then step "SKIP heavy $name"; return 0; fi
  local m; m=$(memavail); [ "$m" -ge "$MIN_KB" ] || { step "ABORT $name: MemAvailable ${m}kB < ${MIN_KB}kB"; exit 3; }
  step "heavy $name start (MemAvailable ${m}kB, min ${MIN_KB}kB, kill ${KILL_KB}kB, jobs ${CARGO_BUILD_JOBS:-})"
  local t0 start=$m; t0=$(date +%s); local low=$m
  setsid "$@" > >(tee "$LOG_DIR/$name.log") 2>&1 & local pid=$!
  while kill -0 $pid 2>/dev/null; do
    m=$(memavail); [ "$m" -lt "$low" ] && low=$m
    if [ "$m" -lt "$KILL_KB" ]; then step "WATCHDOG kill $name: MemAvailable ${m}kB < ${KILL_KB}kB (no retry)"; kill -TERM -- -$pid 2>/dev/null; sleep 5; kill -KILL -- -$pid 2>/dev/null
      printf '%s\t%s\t%s\t%s\t%s\tKILLED\n' "$name" "$(( $(date +%s)-t0 ))" "$start" "$low" "$((start-low))" >> "$LOG_DIR/heavy_stats.tsv"; exit 4; fi
    sleep 1
  done
  local rc=0; wait $pid || rc=$?
  printf '%s\t%s\t%s\t%s\t%s\t%s\n' "$name" "$(( $(date +%s)-t0 ))" "$start" "$low" "$((start-low))" "$rc" >> "$LOG_DIR/heavy_stats.tsv"
  step "heavy $name end rc=$rc $(( $(date +%s)-t0 ))s start=${start}kB min_MemAvailable=${low}kB peak_used=$((start-low))kB"
  [ $rc = 0 ] && return 0
  step "FAIL $name rc=$rc"; [ "${SOFT_FAIL:-0}" = 1 ] && return $rc; exit 5
}
[ -f "$LOG_DIR/heavy_stats.tsv" ] || printf 'step\tseconds\tstart_memavail_kb\tmin_memavail_kb\tpeak_used_kb\trc\n' > "$LOG_DIR/heavy_stats.tsv"
prepend_rc(){ # prepend_rc <marker> <line>: idempotent, top of ~/.profile and ~/.bashrc (before non-interactive early return)
  local rc; for rc in "$HOME/.profile" "$HOME/.bashrc"; do touch "$rc"; grep -qF "$1" "$rc" && continue; { printf '%s\n' "$2"; cat "$rc"; } > "$rc.lcinit" && mv "$rc.lcinit" "$rc"; done
}
step "1/8 system packages"
PKGS="git curl ca-certificates build-essential pkg-config libssl-dev flock xz-utils unzip"
missing=$(for p in git curl gcc g++ make pkg-config flock xz unzip; do command -v $p >/dev/null || echo $p; done)
if [ -n "$missing" ]; then sudo apt-get update -qq && sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq $PKGS util-linux; fi
dpkg -s libssl-dev >/dev/null 2>&1 || sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq libssl-dev
step "2/8 clone $REPO ($BRANCH) -> $DIR"
if [ -d "$DIR/.git" ]; then git -C "$DIR" fetch -q origin "$BRANCH" && git -C "$DIR" checkout -q "$BRANCH" && git -C "$DIR" merge -q --ff-only "origin/$BRANCH"; else git clone -q -b "$BRANCH" "$REPO" "$DIR"; fi
export LC_ROOT="$DIR"; cd "$DIR"
if [ "$LOG_RELOCATE" = 1 ]; then
  NEW="$DIR/.tmp/init-logs"; mkdir -p "$NEW"
  if [ "$LOG_DIR" != "$NEW" ]; then for f in "$LOG_DIR"/*; do [ -e "$f" ] || continue; cat "$f" >> "$NEW/$(basename "$f")" 2>/dev/null || cp -a "$f" "$NEW/"; done; rm -rf "$LOG_DIR"; fi
  LOG_DIR="$NEW"; LOG_RELOCATE=0
  [ -f "$LOG_DIR/heavy_stats.tsv" ] || printf 'step\tseconds\tstart_memavail_kb\tmin_memavail_kb\tpeak_used_kb\trc\n' > "$LOG_DIR/heavy_stats.tsv"
fi
step "3/8 rustup + pinned toolchain (rust-toolchain.toml) into \$LC_ROOT/.litecode/ops/.toolchain"
export RUSTUP_HOME="$DIR/.litecode/ops/.toolchain/rustup" CARGO_HOME="$DIR/.litecode/ops/.toolchain/cargo"; export PATH="$CARGO_HOME/bin:$PATH"
PIN=$(sed -n 's/^channel *= *"\(.*\)"/\1/p' rust-toolchain.toml); [ -n "$PIN" ] || { step "no channel in rust-toolchain.toml"; exit 2; }
[ -x "$CARGO_HOME/bin/rustup" ] || curl -sSf https://sh.rustup.rs | sh -s -- -y --no-modify-path --profile minimal --default-toolchain none
rustup toolchain install "$PIN" --profile minimal -c rustfmt -c clippy; rustup default "$PIN"
ENVLINE="export RUSTUP_HOME=\"$RUSTUP_HOME\" CARGO_HOME=\"$CARGO_HOME\"; export PATH=\"\$CARGO_HOME/bin:\$PATH\"  # litecode init_env"
prepend_rc "# litecode init_env" "$ENVLINE"
step "4/8 node $NODE_MAJOR via fnm"
if ! command -v fnm >/dev/null && [ ! -x "$HOME/.local/share/fnm/fnm" ]; then curl -fsSL https://fnm.vercel.app/install | bash -s -- --skip-shell --install-dir "$HOME/.local/share/fnm"; fi
export PATH="$HOME/.local/share/fnm:$PATH"; eval "$(fnm env --shell bash)"; fnm install "$NODE_MAJOR" >/dev/null; fnm default "$NODE_MAJOR"; fnm use "$NODE_MAJOR" >/dev/null
prepend_rc "# litecode init_env fnm" 'export PATH="$HOME/.local/share/fnm:$PATH"; eval "$(fnm env --shell bash)"  # litecode init_env fnm'
step "versions: $(rustc -V) | $(cargo -V) | node $(node -v) | npm $(npm -v)"
export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-2}
step "5/8 npm ci (web, desktop)"
heavy npm_ci_web npm --prefix web ci --no-audit --no-fund
heavy npm_ci_desktop npm --prefix desktop ci --no-audit --no-fund
step "6/8 cargo build --locked"
heavy cargo_build cargo build --locked
step "7/8 ops toolset"
node .litecode/ops/tools/validate_all.mjs | tee "$LOG_DIR/validate_all.log"
bash -lc "cd '$DIR' && node .litecode/ops/tools/toolchain_check.mjs" | tee "$LOG_DIR/toolchain_check.log"
step "8/8 tests (each test step runs regardless of earlier test failures; memory abort/watchdog still stop immediately)"
SOFT_FAIL=1; TEST_FAILED=()
run_test(){ heavy "$@" || TEST_FAILED+=("$1"); }
run_test ops_tests bash -c 'node --test .litecode/ops/tools/*.test.mjs'
run_test cargo_test cargo test --locked --no-fail-fast
run_test web_test bash -c 'cd web && npx vitest run --maxWorkers=1'
run_test desktop_test npm --prefix desktop test
SOFT_FAIL=0
if [ ${#TEST_FAILED[@]} -gt 0 ]; then
  { echo "failed steps (${#TEST_FAILED[@]}): ${TEST_FAILED[*]}"; node .litecode/ops/tools/test_failures.mjs $(for t in "${TEST_FAILED[@]}"; do printf '%s=%s ' "$t" "$LOG_DIR/$t.log"; done); } | tee "$LOG_DIR/test_failures.txt"
  step "FAIL tests: ${TEST_FAILED[*]} (summary: $LOG_DIR/test_failures.txt)"; exit 6
fi
step "DONE ok; logs in $LOG_DIR"

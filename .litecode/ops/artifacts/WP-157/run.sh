#!/usr/bin/env bash
# WP-157 runner. Usage: run.sh <group: cold-small|cold-big|warm> [N=3]
# Do NOT run until backend_mid has granted the measurement window.
set -euo pipefail
G=$1; N=${2:-3}; HERE=$(cd "$(dirname "$0")" && pwd); REPO=/workspace/litecode
export PLAYWRIGHT_BROWSERS_PATH=$HERE/browsers; NODE=/exec-daemon/node
BIN=${LITECODE_BIN:-$REPO/target/debug/litecode}; PORT=${PORT:-7591}
SMALL=${SMALL_REPO:-$HERE/small-repo}
export LITECODE_WEB_DIST=$REPO/web/dist
mkdir -p "$HERE/raw/$G"
start_serve() { # $1 workspace
  local ws=$1
  read -r wall mono < <($NODE -e 'console.log(Date.now(), (process.hrtime.bigint()/1000000n).toString())')
  ( cd "$ws" && exec "$BIN" serve --bind 127.0.0.1:$PORT --loopback-only ) > "$HERE/raw/$G/serve-$i.log" 2>&1 &
  SERVE_PID=$!
  export SERVE_START_JSON="{\"wall_ms\":$wall,\"mono_ms\":$mono,\"pid\":$SERVE_PID,\"workspace\":\"$ws\"}"
  until grep -q '^LITECODE_READY' "$HERE/raw/$G/serve-$i.log"; do sleep 0.02; kill -0 $SERVE_PID; done
  read -r rwall < <($NODE -e 'console.log(Date.now())'); echo "{\"ready_wall_ms\":$rwall}" > "$HERE/raw/$G/serve-$i.ready.json"
}
stop_serve() { kill $SERVE_PID 2>/dev/null || true; wait $SERVE_PID 2>/dev/null || true; }
case $G in
  cold-small) [ -d "$SMALL/.git" ] || { mkdir -p "$SMALL" && cd "$SMALL" && git init -q && printf 'fn main(){}\n' > main.rs && git add . && git -c user.email=x@x -c user.name=x commit -qm init; }
    for i in $(seq 1 $N); do rm -rf "$SMALL/.litecode"; start_serve "$SMALL"; $NODE "$HERE/probe.mjs" "http://127.0.0.1:$PORT/" "$HERE/raw/$G/run-$i.json" || true; stop_serve; done ;;
  cold-big) for i in $(seq 1 $N); do start_serve "$REPO"; $NODE "$HERE/probe.mjs" "http://127.0.0.1:$PORT/" "$HERE/raw/$G/run-$i.json" || true; stop_serve; done ;;
  warm) i=0; start_serve "$REPO"; $NODE "$HERE/probe.mjs" "http://127.0.0.1:$PORT/" "$HERE/raw/$G/prime.json" || true
    # warm = same serve process; new page, persistent profile so layout + HTTP cache exist
    for i in $(seq 1 $N); do $NODE "$HERE/probe.mjs" "http://127.0.0.1:$PORT/" "$HERE/raw/$G/run-$i.json" --reuse-context-dir "$HERE/profile-warm" || true; done; stop_serve ;;
esac
$NODE "$HERE/median.mjs" "$HERE/raw/$G"

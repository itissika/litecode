#!/bin/bash
# usage: run.sh <workspace> <serve_order|decompose> [runs]  — guarded like the probe.
set -u
B=/workspace/litecode/target/release/wp155_open_harness
for i in $(seq 1 ${3:-3}); do
  if pgrep -x cargo >/dev/null || pgrep -x rustc >/dev/null; then echo "SKIP cargo/rustc"; exit 2; fi
  [ $(awk '/MemAvailable/{print $2}' /proc/meminfo) -lt 6291456 ] && { echo "SKIP memavail"; exit 3; }
  CARGO_BIN_EXE_litecode=/workspace/litecode/target/release/litecode HOME=/workspace/litecode/.tmp/wp154/home-harness LITECODE_HOME=/workspace/litecode/.tmp/wp154/home-harness LITECODE_WEB_DIST=/workspace/litecode/.tmp/wp154/webstub \
  LITECODE_MODEL_DIR=/workspace/litecode/models/ibm-granite/granite-embedding-97m-multilingual-r2 \
    prlimit --as=6442450944 taskset -c 0-3 timeout 900 $B "$1" "$2" </dev/null
  sleep 3
done

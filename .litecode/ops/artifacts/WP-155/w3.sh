#!/bin/bash
# W3: one-time large-repo first build with the production embedder (R4). DO NOT start before the W3 window.
# Guards: no cargo/rustc, MemAvailable>=6GiB (probe refuses otherwise); RSS watchdog 3.5GiB inside the probe; taskset 0-3.
set -u
cd /workspace/litecode/.tmp/wp154
BIG=/workspace/litecode/.tmp/wp154/big
ok() { ! pgrep -x cargo >/dev/null && ! pgrep -x rustc >/dev/null && [ $(awk "/MemAvailable/{print \$2}" /proc/meminfo) -ge 6291456 ]; }
while :; do if ok; then sleep 10; ok && break; fi; sleep 5; done
echo "- $(date +%H:%M:%S) W3 START (MemAvailable $(awk "/MemAvailable/{print \$2}" /proc/meminfo) kB)" >> /workspace/litecode/.litecode/ops/artifacts/WP-155/ide_progress.md
# Side watcher: the first time job.json carries chunks.total (exact, end of pass 1), write it with the time.
( rm -f w3_total.txt
  while [ ! -s w3_total.txt ]; do
    t=$(python3 -c 'import json,sys
try:
  j=json.load(open(sys.argv[1])); v=(j.get("chunks") or {}).get("total")
  print(v if v is not None else "")
except Exception: print("")' "$BIG/.litecode/index/job.json" 2>/dev/null)
    [ -n "$t" ] && { echo "chunks.total=$t at $(date '+%Y-%m-%d %H:%M:%S %Z') ($(date +%s))" > w3_total.txt
                     echo "- $(date +%H:%M:%S) W3 chunks.total=$t (exact; at 7.2 chunks/s reference rate the embed phase would be total/7.2 s)" >> /workspace/litecode/.litecode/ops/artifacts/WP-155/ide_progress.md; }
    sleep 1
  done ) &
W=$!
node /workspace/litecode/.litecode/ops/artifacts/WP-155/probe/run.mjs --bin /workspace/litecode/target/release/litecode --corpus "$BIG" --scenario cold_noindex --profile new \
  --runs 1 --wait-index "$PWD/index-seed-big" --wait-index-ms 14400000 --wait-warm-ms 600000 --out "$PWD/raw/w3_big_firstbuild.jsonl" > w3.out 2>&1
rc=$?
kill $W 2>/dev/null
echo "- $(date +%H:%M:%S) W3 END (probe exit=$rc; seed=$(ls index-seed-big/index 2>/dev/null | wc -l) files)" >> /workspace/litecode/.litecode/ops/artifacts/WP-155/ide_progress.md

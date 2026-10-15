#!/bin/bash
# W4 resume: idempotent; skips runs that already have a record. Guard before every run; one progress line per run.
set -u
cd /workspace/litecode/.tmp/wp154
W=$PWD; BIN=/workspace/litecode/target/release/litecode; R=$W/raw/w4
log() { echo "- $(date +%H:%M:%S) $*" >> /workspace/litecode/.litecode/ops/artifacts/WP-155/ide_progress.md; }
ok() { ! pgrep -x cargo >/dev/null && ! pgrep -x rustc >/dev/null && [ $(awk '/MemAvailable/{print $2}' /proc/meminfo) -ge 6291456 ]; }
guard() { local n=0; until ok; do [ $n -eq 0 ] && log "W4 guard wait ($1)"; n=$((n+1)); sleep 5; [ $n -gt 720 ] && { log "W4 ABORT guard timeout ($1)"; exit 4; }; done; }
for c in small big; do for s in cold_noindex cold_noindex_tc cold_index cold_index_tc hot_reconnect hot_reload; do for i in 1 2 3; do
  d=$R/$c/$s/r$i; [ -s $d/rec.jsonl ] && continue
  rm -rf $d; mkdir -p $d; sc=${s%_tc}; extra=()
  case $sc in hot_*) extra=(--hot-profile restore);; cold_index) extra=(--index-seed $W/index-seed-$c);; esac
  [ $s != $sc ] && extra+=(--true-cold 1)
  for try in 1 2 3; do guard "$c $s r$i"
    node /workspace/litecode/.litecode/ops/artifacts/WP-155/probe/run.mjs --bin $BIN --corpus $W/$c --scenario $sc --profile new --runs 1 --wait-warm-ms 600000 --out $d/rec.jsonl "${extra[@]}" >> $R/probe.out 2>&1
    [ -s $d/rec.jsonl ] && break; log "W4 $c $s r$i refused/failed try $try (probe.out)"; sleep 10; done
  [ -s $d/rec.jsonl ] && log "W4 done $c $s r$i"
done; done; done
for c in small big; do for m in serve_order decompose; do for i in 1 2 3; do
  f=$R/harness/${c}_${m}_r$i.json; [ -s $f ] && continue; mkdir -p $R/harness
  guard "harness $c $m r$i"
  bash /workspace/litecode/.litecode/ops/artifacts/WP-155/harness/run.sh $W/$c $m 1 > $f.tmp 2> ${f%.json}.err; rc=$?
  if grep -q '^{' $f.tmp; then grep '^{' $f.tmp > $f; log "W4 done harness $c $m r$i"; else log "W4 harness $c $m r$i FAILED rc=$rc"; fi; rm -f $f.tmp
done; done; done
log "W4 END"

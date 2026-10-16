#!/bin/bash
# W4 quiet timing. Guard before EVERY run: no cargo/rustc, MemAvailable >= 6GiB.
set -u
cd /workspace/litecode/.tmp/wp154
W=$PWD; BIN=/workspace/litecode/target/release/litecode; R=$W/raw/w4
log() { echo "- $(date +%H:%M:%S) $*" >> /workspace/litecode/.litecode/ops/artifacts/WP-155/ide_progress.md; }
ok() { ! pgrep -x cargo >/dev/null && ! pgrep -x rustc >/dev/null && [ $(awk '/MemAvailable/{print $2}' /proc/meminfo) -ge 6291456 ]; }
guard() { local n=0; until ok; do [ $n -eq 0 ] && log "W4 guard wait ($1)"; n=$((n+1)); sleep 5; [ $n -gt 720 ] && { log "W4 ABORT guard timeout ($1)"; exit 4; }; done; }
probe() { # corpus scen tc i [extra...]
  local c=$1 s=$2 tc=$3 i=$4; shift 4
  local d=$R/$c/$s$([ $tc = 1 ] && echo _tc)/r$i; mkdir -p $d
  local prof=new; local extra=()
  case $s in hot_*) extra=(--hot-profile restore);; esac
  [ $s = cold_index ] && extra+=(--index-seed $W/index-seed-$c)
  [ $tc = 1 ] && extra+=(--true-cold 1)
  guard "$c $s tc=$tc r$i"
  node /workspace/litecode/.litecode/ops/artifacts/WP-155/probe/run.mjs --bin $BIN --corpus $W/$c --scenario $s --profile $prof --runs 1 --wait-warm-ms 600000 --out $d/rec.jsonl "${extra[@]}" "$@" >> $R/probe.out 2>&1 \
    || log "W4 run FAILED $c $s tc=$tc r$i (see raw/w4/probe.out)"
}
mkdir -p $R
log "W4 START (MemAvailable $(awk '/MemAvailable/{print $2}' /proc/meminfo) kB)"
# small-repo index seed (one-time, prerequisite for small cold_index; not a W4 measurement)
if [ ! -d index-seed-small/index ]; then guard seed-small
  node /workspace/litecode/.litecode/ops/artifacts/WP-155/probe/run.mjs --bin $BIN --corpus $W/small --scenario cold_noindex --profile new --runs 1 --wait-index $W/index-seed-small --wait-index-ms 3600000 --out $R/seed_small.jsonl >> $R/probe.out 2>&1
  log "W4 small seed built: $(python3 -c "import json;print(json.load(open('$W/index-seed-small/index/meta.json'))['indexed_chunks'])" 2>&1) chunks"
fi
for c in small big; do
  for i in 1 2 3; do probe $c cold_noindex 0 $i; done
  for i in 1 2 3; do probe $c cold_noindex 1 $i; done
  for i in 1 2 3; do probe $c cold_index 0 $i; done
  for i in 1 2 3; do probe $c cold_index 1 $i; done
  for i in 1 2 3; do probe $c hot_reconnect 0 $i; done
  for i in 1 2 3; do probe $c hot_reload 0 $i; done
  log "W4 $c probe runs done"
done
for c in small big; do for m in serve_order decompose; do
  guard "harness $c $m"; bash /workspace/litecode/.litecode/ops/artifacts/WP-155/harness/run.sh $W/$c $m 3 > $R/harness_${c}_$m.jsonl 2> $R/harness_${c}_$m.err || log "W4 harness $c $m exit=$?"
done; done
log "W4 END"

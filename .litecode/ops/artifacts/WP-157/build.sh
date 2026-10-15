#!/bin/bash
m(){ awk '/MemAvailable/{print $2}' /proc/meminfo; }
for i in $(seq 1 360); do [ $(m) -ge 6291456 ] && break; sleep 5; done
[ $(m) -ge 6291456 ] || { echo "GATE_FAIL $(m)"; exit 3; }
echo "START $(date +%T) mem=$(m)"
( /exec-daemon/node node_modules/typescript/bin/tsc -b && /exec-daemon/node node_modules/vite/bin/vite.js build ) > /tmp/wp157-build.log 2>&1 &
P=$!; min=$(m)
while kill -0 $P 2>/dev/null; do v=$(m); [ $v -lt $min ] && min=$v; if [ $v -lt 2097152 ]; then pkill -P $P; kill $P; echo "ABORT mem=$v"; exit 4; fi; sleep 2; done
wait $P; rc=$?; echo "END $(date +%T) rc=$rc minMem=$min"; tail -5 /tmp/wp157-build.log

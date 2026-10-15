#!/usr/bin/env bash
# LiteCode ops: run cargo with the shared env, serialized on .tmp/cargo-target.lock.
# Usage: cargo_locked.sh [-t SECS] <cargo args...>   (default timeout 3000s; stdin </dev/null)
# A private CARGO_TARGET_DIR (outside /workspace/litecode/target) still takes the lock (memory guard: one heavy cargo at a time).
set -euo pipefail
t=3000; if [ "${1:-}" = "-t" ]; then t=$2; shift 2; fi
source "$(dirname "$(readlink -f "$0")")/cargo_env.sh"
cd /workspace/litecode
exec flock "$LC_CARGO_LOCK" timeout "$t" cargo "$@" </dev/null

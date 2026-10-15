#!/usr/bin/env bash
# LiteCode ops: run cargo with the shared env, serialized on .tmp/cargo-target.lock.
# Usage: cargo_locked.sh [-t SECS] <cargo args...>   (default timeout 3000s; stdin </dev/null)
# A private CARGO_TARGET_DIR (outside /workspace/litecode/target) still takes the lock (memory guard: one heavy cargo at a time).
set -euo pipefail
t=3000; if [ "${1:-}" = "-t" ]; then t=$2; shift 2; fi
source "$(dirname "$(readlink -f "$0")")/cargo_env.sh"
cd /workspace/litecode
# v20: quick registry check (stat-only, ~0.2s healthy); only if damage found, auto --fix and print ONE line. LC_SKIP_VERIFY=1 bypasses.
if [ -z "${LC_SKIP_VERIFY:-}" ]; then
  if ! "$(dirname "$(readlink -f "$0")")/registry_verify.sh" --quick >/dev/null 2>&1; then
    if "$(dirname "$(readlink -f "$0")")/registry_verify.sh" --fix >/tmp/registry_verify_autofix.log 2>&1; then
      echo "cargo_locked: registry damage found and repaired ($(grep -c '^DAMAGED' /tmp/registry_verify_autofix.log) crate(s); log /tmp/registry_verify_autofix.log)" >&2
    else echo "cargo_locked: registry damage NOT fully repaired, see /tmp/registry_verify_autofix.log" >&2; fi
  fi
fi
exec flock "$LC_CARGO_LOCK" timeout "$t" cargo "$@" </dev/null

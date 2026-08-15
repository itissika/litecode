#!/usr/bin/env bash
set -euo pipefail

name="$1"
manifest="$(cd "$(dirname "$0")/../../.." && pwd)"
lock="${TMPDIR:-/tmp}/litecode_external_hook_test.lock"
real="${manifest}/tests/fixtures/hooks/${name}.sh"

if [[ ! -x "$real" ]]; then
  echo "missing hook fixture: $real" >&2
  exit 1
fi

touch "$lock"
exec flock -x "$lock" "$real"

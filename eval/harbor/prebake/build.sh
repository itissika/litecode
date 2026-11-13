#!/usr/bin/env bash
# Build the per-task harness images used by the five-harness panel.
#
# Two paths:
#   * canonical: FROM the task's public image, build Dockerfile.toolchain, then
#     add the grok layer with Dockerfile.harness.
#   * fast: set EVAL_PREBAKE_BASE_<TASK> to an already verified toolchain image
#     (the pre1/pre2 images used for the 2026-11-13 run) and only rebuild the
#     thin harness layer.
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)
here="$repo_root/eval/harbor/prebake"
tag_prefix=${EVAL_PREBAKE_TAG_PREFIX:-litecode-eval}
tag_suffix=${EVAL_PREBAKE_TAG_SUFFIX:-bench5}

record="$here/images.txt"
if [ "${EVAL_PREBAKE_RESET_RECORD:-0}" = "1" ]; then
  : > "$record"
fi
touch "$record"
{
  echo "# built_at $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "# tag_prefix $tag_prefix"
  echo "# tag_suffix $tag_suffix"
} >> "$record"

if [ "$#" -gt 0 ]; then
  tasks=("$@")
else
  tasks=(prove-plus-comm polyglot-c-py password-recovery cancel-async-tasks)
fi

for task in "${tasks[@]}"; do
  env_name="EVAL_PREBAKE_BASE_$(printf '%s' "$task" | tr '[:lower:]-' '[:upper:]_')"
  fast_base=${!env_name:-}
  image="$tag_prefix/$task:$tag_suffix"
  if [ -n "$fast_base" ]; then
    toolchain="$fast_base"
    echo "=== $task: fast path from $toolchain ==="
  else
    task_base=${EVAL_PREBAKE_TASK_BASE:-alexgshaw/$task:20251031}
    toolchain="$tag_prefix/$task:toolchain-$tag_suffix"
    echo "=== $task: canonical toolchain FROM $task_base ==="
    docker build \
      --build-arg "BASE_IMAGE=$task_base" \
      -f "$here/Dockerfile.toolchain" \
      -t "$toolchain" \
      "$here"
  fi

  echo "=== $task: harness layer -> $image ==="
  docker build \
    --build-arg "BASE_IMAGE=$toolchain" \
    -f "$here/Dockerfile.harness" \
    -t "$image" \
    "$here"
  digest=$(docker image inspect "$image" --format '{{.Id}}')
  versions=$(docker run --rm --entrypoint bash "$image" -lc '
    set -e
    . /root/.nvm/nvm.sh
    printf "node=%s\n" "$(node --version)"
    printf "codex=%s\n" "$(codex --version | head -1)"
    printf "pi=%s\n" "$(pi --version | head -1)"
    printf "opencode=%s\n" "$(opencode --version | head -1)"
    printf "grok=%s\n" "$(PATH=/root/.grok/bin:$PATH grok --version | head -1)"
  ')
  {
    echo "image=$image"
    echo "toolchain=$toolchain"
    echo "digest=$digest"
    printf '%s\n' "$versions" | sed 's/^/  /'
  } >> "$record"
  echo "built $image ($digest)"
done

cat "$record"
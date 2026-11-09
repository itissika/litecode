#!/usr/bin/env bash
# Build the Linux release binary plus a credential-free global database, both
# into eval/harbor/bin/. The task container only ever receives those two files.
#
# The tree this runs in has to be a git checkout: every artifact records the
# commit it came from in build-info.json, next to the binary.
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
out="$repo_root/eval/harbor/bin"
stage=$(mktemp -d)
container=""
trap 'docker rm -f "$container" >/dev/null 2>&1 || true; rm -rf "$stage"' EXIT

# A copied directory cannot be traced to a commit, and an artifact without one
# is useless in a results table.
if ! git -C "$repo_root" rev-parse --verify HEAD >/dev/null 2>&1; then
  echo "build.sh: $repo_root is not a git checkout; refusing to build an untraceable binary" >&2
  exit 1
fi
commit=$(git -C "$repo_root" rev-parse HEAD)
branch=$(git -C "$repo_root" rev-parse --abbrev-ref HEAD)
if [ -n "$(git -C "$repo_root" status --porcelain)" ]; then
  dirty=true
else
  dirty=false
fi
built_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)

# Staged context: the crate's inputs only. The Windows working tree holds
# gigabytes of dev artifacts that must not travel to the daemon.
cp "$repo_root/rust-toolchain.toml" "$repo_root/Cargo.toml" "$repo_root/Cargo.lock" \
   "$repo_root/build.rs" "$stage/"
cp -r "$repo_root/src" "$stage/src"
mkdir -p "$stage/sdk"
cp -r "$repo_root/sdk/generated" "$stage/sdk/generated"
cp "$repo_root/eval/harbor/Dockerfile" "$stage/Dockerfile"

tag=${LITECODE_EVAL_IMAGE:-litecode-eval-build}
docker build -f "$stage/Dockerfile" -t "$tag" "$stage"

mkdir -p "$out" "$stage/seed"
container=$(docker create "$tag")
docker cp "$container:/usr/local/bin/litecode" "$out/litecode"
chmod +x "$out/litecode"

# The schema the run will use, created by the app's own migration. No
# credentials go in here: the adapter adds exactly one row at install time.
docker run --rm --entrypoint bash --user "$(id -u):$(id -g)" \
  -v "$stage/seed:/seed" "$tag" -lc \
  'HOME=/seed litecode config set log.level info' >/dev/null
# The app keeps its database in WAL mode and leaves the write in the -wal file,
# so both files travel; the adapter folds them back together.
seed_dir="$stage/seed/.local/share/litecode"
cp "$seed_dir/litecode.db" "$out/litecode-seed.db"
if [ -f "$seed_dir/litecode.db-wal" ]; then
  cp "$seed_dir/litecode.db-wal" "$out/litecode-seed.db-wal"
fi
chmod 644 "$out"/litecode-seed.db*

# The compiler that actually produced the binary: re-tagging the build stage is
# a cache hit, so this costs seconds.
toolchain_tag="$tag-toolchain"
docker build --target build -t "$toolchain_tag" -f "$stage/Dockerfile" "$stage" >/dev/null
rustc_version=$(docker run --rm --entrypoint rustc "$toolchain_tag" -V)

cat > "$out/build-info.json" <<JSON
{
  "commit": "$commit",
  "branch": "$branch",
  "dirty": $dirty,
  "built_at": "$built_at",
  "rustc": "$rustc_version",
  "source_tree": "$repo_root"
}
JSON

if [ "$dirty" = true ]; then
  echo "built $out/litecode at $commit (uncommitted changes)"
else
  echo "built $out/litecode at $commit"
fi
echo "wrote  $out/build-info.json"

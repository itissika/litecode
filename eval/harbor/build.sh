#!/usr/bin/env bash
# Build the Linux release binary plus a credential-free global database, both
# into eval/harbor/bin/. The task container only ever receives those two files.
#
# The tree this runs in has to be a git checkout: every artifact records the
# commit it came from in build-info.json, next to the binary.
#
# LITECODE_EVAL_PORTABLE=1 builds on Debian 12 with ORT as load-dynamic. It is
# the eval mode for Debian-bookworm task images: GLIBC_2.35, works on both
# Debian 12 and Ubuntu 24.04. Without it the normal Ubuntu 24.04 build is used.
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
portable=${LITECODE_EVAL_PORTABLE:-0}
if [ "$portable" = "1" ]; then
  build_base=${LITECODE_BUILD_BASE_IMAGE:-debian:12-slim}
  portable_ort=load-dynamic
  cargo_locked=false
else
  build_base=${LITECODE_BUILD_BASE_IMAGE:-ubuntu:24.04}
  portable_ort=download-binaries
  cargo_locked=true
fi
if [ "$portable" = "1" ]; then
  # ORT's downloaded archive requires GLIBC_2.38+. Eval profiles disable
  # code_search/session_search, so a load-dynamic build is sufficient there.
  sed -i 's/"download-binaries"/"load-dynamic"/' "$stage/Cargo.toml"
  sed -i 's/"load-dynamic",/"load-dynamic", "api-24",/' "$stage/Cargo.toml"
  sed -i 's/cargo build --release --locked/cargo build --release/' "$stage/Dockerfile"
fi
docker build --build-arg "BUILD_BASE_IMAGE=$build_base" -f "$stage/Dockerfile" -t "$tag" "$stage"

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
docker build --target build --build-arg "BUILD_BASE_IMAGE=$build_base" -t "$toolchain_tag" -f "$stage/Dockerfile" "$stage" >/dev/null
rustc_version=$(docker run --rm --entrypoint rustc "$toolchain_tag" -V)

cat > "$out/build-info.json" <<JSON
{
  "commit": "$commit",
  "branch": "$branch",
  "dirty": $dirty,
  "built_at": "$built_at",
  "rustc": "$rustc_version",
  "build_base": "$build_base",
  "portable_ort": "$portable_ort",
  "cargo_locked": $cargo_locked,
  "source_tree": "$repo_root"
}
JSON

if [ "$dirty" = true ]; then
  echo "built $out/litecode at $commit (uncommitted changes)"
else
  echo "built $out/litecode at $commit"
fi
echo "wrote  $out/build-info.json"

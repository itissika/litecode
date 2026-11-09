#!/usr/bin/env bash
# Build the Linux release binary plus a credential-free global database, both
# into eval/harbor/bin/. The task container only ever receives those two files.
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
out="$repo_root/eval/harbor/bin"
stage=$(mktemp -d)
container=""
trap 'docker rm -f "$container" >/dev/null 2>&1 || true; rm -rf "$stage"' EXIT

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

echo "built $out/litecode"
echo "built $out/litecode-seed.db"

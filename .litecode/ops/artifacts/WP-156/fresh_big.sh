#!/bin/sh
# C2 prep: reset the large-repo copy to "code seed, no session index" (fresh seed copy + corpus copy + engines.json).
# The seed original is only copied, never modified. Paths are outside the repo (WP156_DIR) except the read-only seed.
set -e
W=${WP156_DIR:-/workspace/wp156-recover/run}
B=$W/big
SEED=${WP156_SEED:-/workspace/litecode/.tmp/wp154/index-seed-big/index}
rm -rf "$B/.litecode"
mkdir -p "$B/.litecode"
cp -a "$SEED" "$B/.litecode/index"
cp "$W/corpus_big.db" "$B/.litecode/sessions.db"
cp "$W/engines_big.json" "$B/.litecode/engines.json"   # C2 real config: retrieval on, LSP rust-analyzer on

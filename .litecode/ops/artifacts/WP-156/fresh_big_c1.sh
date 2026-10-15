#!/bin/sh
# C1 (method.md): fresh process + first open: no .litecode except, for path B, the corpus copy.
# No code seed, no engines.json (retrieval/LSP off -> no embed). usage: fresh_big_c1.sh A|B
set -e
W=${WP156_DIR:-/workspace/wp156-recover/run}
B=$W/big
rm -rf "$B/.litecode"
if [ "$1" = B ]; then mkdir -p "$B/.litecode"; cp "$W/corpus_big.db" "$B/.litecode/sessions.db"; fi

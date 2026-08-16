#!/usr/bin/env bash
# Copy the locked ORT WOQ embed bundle into the product models/ tree.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# Default: bundle from the already-bundled tree in the repo (idempotent).
# Override to rebuild from a local eval source, e.g.
#   LITECODE_BUNDLE_SRC="$ROOT/dev/eval/retrieval/.data/models/ibm-granite/granite-embedding-97m-multilingual-r2"
SRC="${LITECODE_BUNDLE_SRC:-$ROOT/models/ibm-granite/granite-embedding-97m-multilingual-r2}"
DEST="$ROOT/models/ibm-granite/granite-embedding-97m-multilingual-r2"

ART="$SRC/artifacts/ort-lin-q8-emb-q4-bs128-a1.onnx"
if [[ ! -f "$ART" ]]; then
  echo "source WOQ artifact missing: $ART" >&2
  echo "set LITECODE_BUNDLE_SRC to a tree that contains the ort-lin-q8-emb-q4-bs128-a1 artifact (see dev/eval build docs)" >&2
  exit 1
fi

mkdir -p "$DEST/artifacts" "$DEST/1_Pooling"
cp -f "$SRC/config.json" "$DEST/"
cp -f "$SRC/tokenizer.json" "$DEST/"
cp -f "$SRC/1_Pooling/config.json" "$DEST/1_Pooling/"
cp -f "$SRC/artifacts/ort-lin-q8-emb-q4-bs128-a1.onnx" "$DEST/artifacts/"
cp -f "$SRC/artifacts/ort-lin-q8-emb-q4-bs128-a1.onnx.data" "$DEST/artifacts/"
if [[ -f "$SRC/artifacts/ort-lin-q8-emb-q4-bs128-a1.SOURCE.json" ]]; then
  cp -f "$SRC/artifacts/ort-lin-q8-emb-q4-bs128-a1.SOURCE.json" "$DEST/artifacts/"
fi

echo "bundled embed model -> $DEST"

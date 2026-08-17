#!/usr/bin/env bash
# Assemble a cwd-independent product tree for smoke / future installers.
# Does not build Electron.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="${LITECODE_PRODUCT_DIR:-$ROOT/dist/product}"
TARGET_TRIPLE="${LITECODE_TARGET:-}"
BUILD_WEB="${LITECODE_BUILD_WEB:-1}"
BUNDLE_MODEL="${LITECODE_BUNDLE_MODEL:-1}"
PROFILE="${LITECODE_PROFILE:-release}"
CARGO_PROFILE_FLAG="--release"
PROFILE_DIR="release"
if [[ "$PROFILE" == "debug" ]]; then
  CARGO_PROFILE_FLAG=""
  PROFILE_DIR="debug"
fi

echo "==> product root: $OUT (profile=$PROFILE)"
rm -rf "$OUT"
mkdir -p "$OUT"

if [[ "$BUILD_WEB" == "1" ]]; then
  echo "==> building web/dist"
  (cd "$ROOT/web" && npm run build)
fi

if [[ ! -d "$ROOT/web/dist" ]]; then
  echo "web/dist missing; run npm run build in web/ or set LITECODE_BUILD_WEB=1" >&2
  exit 1
fi

if [[ "$BUNDLE_MODEL" == "1" ]]; then
  echo "==> bundling embed model"
  "$ROOT/scripts/bundle_embed_model.sh"
  if [[ ! -d "$ROOT/models/ibm-granite/granite-embedding-97m-multilingual-r2" ]]; then
    echo "models/ bundle missing; run scripts/bundle_embed_model.sh" >&2
    exit 1
  fi
else
  echo "==> skipping embed model (LITECODE_BUNDLE_MODEL=0, slim SKU)"
fi

echo "==> cargo build ${CARGO_PROFILE_FLAG}${TARGET_TRIPLE:+ --target $TARGET_TRIPLE}"
# Release CI sets official; local product assembly is always nightly otherwise.
if [[ "${LITECODE_CHANNEL:-}" != "official" ]]; then
  export LITECODE_CHANNEL=nightly
fi
TARGET_ROOT="${CARGO_TARGET_DIR:-$ROOT/target}"
if [[ -n "$TARGET_TRIPLE" ]]; then
  # shellcheck disable=SC2086
  (cd "$ROOT" && cargo build $CARGO_PROFILE_FLAG --target "$TARGET_TRIPLE")
  BIN_DIR="$TARGET_ROOT/$TARGET_TRIPLE/$PROFILE_DIR"
else
  # shellcheck disable=SC2086
  (cd "$ROOT" && cargo build $CARGO_PROFILE_FLAG)
  BIN_DIR="$TARGET_ROOT/$PROFILE_DIR"
fi

BIN_NAME="litecode"
if [[ -f "$BIN_DIR/litecode" ]]; then
  BIN_NAME="litecode"
elif [[ -f "$BIN_DIR/litecode.exe" ]]; then
  BIN_NAME="litecode.exe"
fi
if [[ ! -f "$BIN_DIR/$BIN_NAME" ]]; then
  echo "missing $BIN_DIR/$BIN_NAME" >&2
  exit 1
fi

cp -f "$BIN_DIR/$BIN_NAME" "$OUT/"
# ORT / native dylibs copied next to the binary by ort's copy-dylibs feature.
shopt -s nullglob
for f in "$BIN_DIR"/*.dll "$BIN_DIR"/*.dylib "$BIN_DIR"/*.so "$BIN_DIR"/lib*.so*; do
  cp -f "$f" "$OUT/" 2>/dev/null || true
done
shopt -u nullglob

mkdir -p "$OUT/web"
cp -a "$ROOT/web/dist" "$OUT/web/"
if [[ "$BUNDLE_MODEL" == "1" ]]; then
  mkdir -p "$OUT/models"
  cp -a "$ROOT/models/." "$OUT/models/"
fi

cat > "$OUT/README.txt" <<EOF
Litecode product layout (sidecar-ready)

  ./$BIN_NAME serve --bind 127.0.0.1:0 --require-auth
  # set LITECODE_TOKEN in the environment (host-injected; users never type it)

  # Cloud / remote (non-loopback requires --require-auth + token):
  #   ./$BIN_NAME --workspace /path/to/repo serve --bind 0.0.0.0:7483 --require-auth

Layout:
  $BIN_NAME     kernel binary
  web/dist/     UI (served by litecode)
  models/       embedding weights when bundled (full SKU)
  *.dll/*.so    native runtime deps when present

Global settings DB is created on first run under the OS user data dir
(Windows: %LOCALAPPDATA%\\litecode; Unix: ~/.local/share/litecode).
Per-workspace data lives in <workspace>/.litecode/.
EOF

echo "==> done: $OUT"
ls -la "$OUT"

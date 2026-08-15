#!/usr/bin/env bash
# Package litecode-server-*-linux-x64.tar.gz (headless remote kernel).
# Intended for native Linux or WSL self-test — not the Windows Electron package.
# Formal release CI (ubuntu-latest) is postposed; WSL is not release authority.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PROFILE="${LITECODE_PROFILE:-release}"
BUNDLE_MODEL="${LITECODE_BUNDLE_MODEL:-1}"
VERSION="${LITECODE_VERSION:-}"

if [[ -z "$VERSION" ]]; then
  VERSION="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$ROOT/Cargo.toml" | head -n1)"
  VERSION="${VERSION//$'\r'/}"
fi
if [[ -z "$VERSION" ]]; then
  echo "could not resolve LITECODE_VERSION / Cargo.toml version" >&2
  exit 1
fi

STAGE="${LITECODE_PRODUCT_DIR:-$ROOT/dist/litecode-server-stage}"
DIST_DIR="$ROOT/dist"
TAR_NAME="litecode-server-${VERSION}-linux-x64.tar.gz"
TAR_PATH="$DIST_DIR/$TAR_NAME"

echo "==> packaging $TAR_NAME (profile=$PROFILE bundle_model=$BUNDLE_MODEL)"

export LITECODE_PROFILE="$PROFILE"
export LITECODE_BUNDLE_MODEL="$BUNDLE_MODEL"
export LITECODE_PRODUCT_DIR="$STAGE"
# package_linux always rebuilds for a clean tree unless caller sets BUILD_WEB=0
export LITECODE_BUILD_WEB="${LITECODE_BUILD_WEB:-1}"

"$ROOT/scripts/assemble_product.sh"

BIN="$STAGE/litecode"
if [[ ! -x "$BIN" && ! -f "$BIN" ]]; then
  echo "missing $BIN after assemble" >&2
  exit 1
fi

cat > "$STAGE/README.txt" <<EOF
Litecode Linux server (headless)

Typical cloud start (2C2G personal VPS; features are not stripped — watch memory):

  export LITECODE_TOKEN="\$(openssl rand -base64 32)"
  ./litecode --workspace /path/to/repo serve \\
    --bind 0.0.0.0:7483 \\
    --require-auth

Connect from Windows Electron (product path):
  Options → Connect to remote… → Base URL + the same token.

Recommended install when the cloud network is flaky:
  Directly curl/wget the tar on the server, extract and start as above.

Layout:
  litecode      kernel binary
  web/dist/     UI (served by litecode)
  models/       present on full SKU (LITECODE_BUNDLE_MODEL=1)
  *.so          native runtime deps when present

See scripts/litecode.service.example.
EOF

mkdir -p "$DIST_DIR"
rm -f "$TAR_PATH" "${TAR_PATH}.sha256"
# Archive contents at top level of the tar (litecode, web/, …)
tar -C "$STAGE" -czf "$TAR_PATH" .

if command -v sha256sum >/dev/null 2>&1; then
  (cd "$DIST_DIR" && sha256sum "$TAR_NAME" > "${TAR_NAME}.sha256")
elif command -v shasum >/dev/null 2>&1; then
  (cd "$DIST_DIR" && shasum -a 256 "$TAR_NAME" > "${TAR_NAME}.sha256")
else
  echo "warning: no sha256sum/shasum; skipped .sha256" >&2
fi

# Stable embed path for Windows Electron (dev + electron-builder extraResources).
LINUX_STAGE="$DIST_DIR/linux"
STABLE_TAR="$LINUX_STAGE/litecode-server-linux-x64.tar.gz"
mkdir -p "$LINUX_STAGE"
cp -f "$TAR_PATH" "$STABLE_TAR"
cp -f "${TAR_PATH}.sha256" "${STABLE_TAR}.sha256"

echo "==> wrote $TAR_PATH"
echo "==> staged $STABLE_TAR"
ls -lh "$TAR_PATH" "${TAR_PATH}.sha256" "$STABLE_TAR" "${STABLE_TAR}.sha256" 2>/dev/null || ls -lh "$TAR_PATH" "$STABLE_TAR"

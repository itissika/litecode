#!/usr/bin/env bash
# Build one relocatable harness runtime for the shared-mount eval model.
#
# The task image stays the task image.  This directory carries Node and the
# external CLIs (codex, pi, opencode); Harbor bind-mounts it read-only at
# /opt/harness, and the adapter only adds a PATH shim inside the trial.
#
# The build runs in Debian 12 so the runtime is linked against the oldest
# glibc in the current task set (Ubuntu 24.04 is newer).  It is deliberately
# not a task image and contains no benchmark files.
#
# Usage:
#   bash eval/harbor/runtime/build.sh
#   OUT=/path/to/runtime bash eval/harbor/runtime/build.sh
#   bash eval/harbor/runtime/build.sh --force
#
# Network access happens inside the Docker build container, not on the WSL
# host.  The output defaults to ~/eval/runtime/<versioned-name>; a "current"
# symlink is updated next to it.

set -euo pipefail

NODE_VERSION="${NODE_VERSION:-22.23.5}"
CODEX_VERSION="${CODEX_VERSION:-0.167.2}"
PI_VERSION="${PI_VERSION:-1.5.2}"
OPENCODE_VERSION="${OPENCODE_VERSION:-1.19.5}"
RG_VERSION="${RG_VERSION:-14.1.1}"
NAME="codex-pi-opencode-node${NODE_VERSION}-c${CODEX_VERSION}-p${PI_VERSION}-o${OPENCODE_VERSION}"
OUT="${OUT:-$HOME/eval/runtime/$NAME}"
FORCE=0

usage() {
  sed -n '2,30p' "$0" | sed 's/^# {0,1}//'
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --out)
      [ "$#" -ge 2 ] || { echo "--out needs a path" >&2; exit 2; }
      OUT="$2"
      shift 2
      ;;
    --force)
      FORCE=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

mkdir -p "$OUT"
OUT="$(cd "$OUT" && pwd)"
if [ -f "$OUT/VERSION" ] && [ "$FORCE" -eq 0 ]; then
  echo "runtime already exists: $OUT"
  echo "use --force to rebuild"
  exit 0
fi
if [ "$FORCE" -eq 1 ]; then
  find "$OUT" -mindepth 1 -maxdepth 1 -exec rm -rf {} +
fi

echo "building runtime: $OUT"
docker run --rm \
  -e NODE_VERSION="$NODE_VERSION" \
  -e CODEX_VERSION="$CODEX_VERSION" \
  -e PI_VERSION="$PI_VERSION" \
  -e OPENCODE_VERSION="$OPENCODE_VERSION" \
  -e RG_VERSION="$RG_VERSION" \
  -v "$OUT:/out" \
  debian:12-slim bash -lc '
    set -euo pipefail
    export DEBIAN_FRONTEND=noninteractive
    apt-get update
    apt-get install -y --no-install-recommends ca-certificates curl xz-utils
    rm -rf /var/lib/apt/lists/*

    node_archive="node-v${NODE_VERSION}-linux-x64.tar.xz"
    curl -fsSL --retry 3 --retry-delay 2 \
      "https://npmmirror.com/mirrors/node/v${NODE_VERSION}/${node_archive}" \
      -o /tmp/${node_archive}
    tar -xJf "/tmp/${node_archive}" -C /out
    mv "/out/node-v${NODE_VERSION}-linux-x64" /out/node
    mkdir -p /out/bin
    ln -sfn ../node/bin/node /out/bin/node
    ln -sfn ../node/bin/npm /out/bin/npm
    ln -sfn ../node/bin/npx /out/bin/npx

    export PATH="/out/bin:$PATH"
    npm config set registry https://registry.npmmirror.com --global
    npm install -g --prefix /out \
      "@openai/codex@${CODEX_VERSION}" \
      "@earendil-works/pi-coding-agent@${PI_VERSION}" \
      "opencode-ai@${OPENCODE_VERSION}"

    rg_archive="ripgrep-${RG_VERSION}-x86_64-unknown-linux-musl.tar.gz"
    curl -fsSL --retry 3 --retry-delay 2 \
      "https://github.com/BurntSushi/ripgrep/releases/download/${RG_VERSION}/${rg_archive}" \
      -o "/tmp/${rg_archive}"
    tar -xzf "/tmp/${rg_archive}" -C /tmp
    install -m 0755 \
      "/tmp/ripgrep-${RG_VERSION}-x86_64-unknown-linux-musl/rg" \
      /out/bin/rg

    npm cache clean --force || true
    rm -rf /root/.npm /tmp/node-v${NODE_VERSION}-linux-x64.tar.xz \
      /tmp/ripgrep-${RG_VERSION}-x86_64-unknown-linux-musl.tar.gz \
      /tmp/ripgrep-${RG_VERSION}-x86_64-unknown-linux-musl
    chmod -R a+rX /out
    chmod 0755 /out/bin/node /out/bin/npm /out/bin/npx /out/bin/rg \
      /out/bin/codex /out/bin/pi /out/bin/opencode
  '

{
  echo "node=${NODE_VERSION}"
  echo "codex=${CODEX_VERSION}"
  echo "pi=${PI_VERSION}"
  echo "opencode=${OPENCODE_VERSION}"
  echo "ripgrep=${RG_VERSION}"
  echo "base=debian:12-slim"
  echo "built_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
} > "$OUT/VERSION"
chmod 0644 "$OUT/VERSION"

runtime_root="$(dirname "$OUT")"
ln -sfn "$(basename "$OUT")" "$runtime_root/current"

echo "smoke testing: $OUT"
for image in debian:12-slim ubuntu:24.04; do
  echo "  $image"
  docker run --rm -e HOME=/tmp \
    -v "$OUT:/opt/harness:ro" \
    --entrypoint bash "$image" -lc '
      set -e
      export PATH=/opt/harness/bin:$PATH
      node --version
      codex --version
      pi --version
      opencode --version
      rg --version | head -1
    '
done

echo "runtime ready: $OUT"
du -sh "$OUT"

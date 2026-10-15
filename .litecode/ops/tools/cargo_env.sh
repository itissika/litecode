# LiteCode ops: shared cargo build env. Usage: source /workspace/litecode/.litecode/ops/tools/cargo_env.sh
# Sets absolute HOME (cargo 1.99 panics on relative HOME), ops toolchain RUSTUP_HOME/CARGO_HOME, shared target dir.
LC_ROOT=/workspace/litecode
unset RUSTUP_HOME CARGO_HOME RUSTUP_TOOLCHAIN
export HOME="$LC_ROOT/.tmp/home"; mkdir -p "$HOME"
export RUSTUP_HOME="$LC_ROOT/.litecode/ops/.toolchain/rustup"
export CARGO_HOME="$LC_ROOT/.litecode/ops/.toolchain/cargo"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$LC_ROOT/target}"
export LC_CARGO_LOCK="$LC_ROOT/.tmp/cargo-target.lock"
case ":$PATH:" in *":$CARGO_HOME/bin:"*) ;; *) export PATH="$CARGO_HOME/bin:$PATH";; esac
# v20: user-space g++ fallback (no root after reboot). Silent no-op when system g++ exists; otherwise use .toolchain/gxx-bin
# (created by tools/registry_verify.sh --fix from cached debs, offline). link-cplusplus needs a C++ driver.
if { [ -n "$LC_FORCE_USERSPACE_GXX" ] || ! command -v g++ >/dev/null 2>&1; } && [ -x "$LC_ROOT/.litecode/ops/.toolchain/gxx-bin/g++" ]; then
  case ":$PATH:" in *":$LC_ROOT/.litecode/ops/.toolchain/gxx-bin:"*) ;; *) export PATH="$LC_ROOT/.litecode/ops/.toolchain/gxx-bin:$PATH";; esac
  export CXX="$LC_ROOT/.litecode/ops/.toolchain/gxx-bin/g++"
fi

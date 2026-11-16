# Shared harness runtime

One relocatable directory holds Node and the external CLIs used by the bench:

| package | binary | version |
|---|---|---|
| Node.js | `node` | 22.23.5 |
| `@openai/codex` | `codex` | 0.167.2 |
| `@earendil-works/pi-coding-agent` | `pi` | 1.5.2 |
| `opencode-ai` | `opencode` | 1.19.5 |
| ripgrep (static musl) | `rg` | 14.1.1 |

LiteCode is **not** in this runtime: its adapter uploads the prebuilt binary
into the trial, exactly as before.

## Build

Run from WSL (Docker lives there):

```bash
bash eval/harbor/runtime/build.sh
```

The output defaults to `~/eval/runtime/codex-pi-opencode-node...`, and a
`current` symlink is updated beside it. The script builds inside Debian 12
so the runtime is linked against the oldest glibc in the current task set,
then smoke-tests it in both `debian:12-slim` and `ubuntu:24.04`.

Override versions or output when needed:

```bash
NODE_VERSION=22.23.5 OUT=/data/runtime/harness bash eval/harbor/runtime/build.sh
```

Rebuild with `--force`. The build downloads from npmmirror/GitHub inside the
build container; the WSL host itself does not need egress. Expect about 1.2GB
on disk: codex and opencode ship native binaries, and opencode includes both
the AVX2 and baseline x64 variants. The first build can take ~45 minutes on
this WSL host; most of that is downloading the two large native tarballs.

## How it is mounted

The campaign points at the runtime directory:

```yaml
harness_runtime: ${HOME}/eval/runtime/current
```

`resolve.py` injects a read-only bind mount:

```yaml
environment:
  mounts:
    - type: bind
      source: /home/sika/eval/runtime/current
      target: /opt/harness
      read_only: true
```

The prebaked adapters write a small `~/.nvm/nvm.sh` shim that prepends
`/opt/harness/bin` to `PATH`; the stock Codex/Pi/OpenCode run commands then
find the shared binaries without an nvm install. The runtime is read-only;
CLI config and caches stay in the trial's `$HOME`.

## Compatibility

The current task set is Ubuntu 24.04 + Debian 12 (glibc). The runtime is built
on Debian 12 to cover both. Alpine/musl, ARM and non-glibc images need a
separate runtime; the runner's runtime preflight rejects an incompatible
mount rather than producing a missing trial.

The task image must provide `bash`, `grep`, `stdbuf`, `tee` and normally
`git`; `rg` is bundled here. If a task image lacks one, the adapter's install
check fails before the model is called.

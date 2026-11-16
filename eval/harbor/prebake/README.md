# Prebaked harness images (legacy)

> **2026-11-14:** the active bench no longer builds per-task harness images.
> It uses one shared runtime mounted at `/opt/harness`; see
> [`../runtime/README.md`](../runtime/README.md). This directory remains as a
> fallback for the old five-harness image model.

Three task images, one per benchmark problem, each carrying the same toolchain:

| task | image |
|---|---|
| prove-plus-comm | `litecode-eval/prove-plus-comm:bench5` |
| polyglot-c-py | `litecode-eval/polyglot-c-py:bench5` |
| password-recovery | `litecode-eval/password-recovery:bench5` |

Each image contains Node 22 and:

| harness | version |
|---|---|
| codex | 0.167.2 |
| pi | 1.5.2 |
| opencode | 1.19.5 |
| grok-build | 1.0.77 |

`images.txt` records the exact image digest and CLI versions produced by the
run. The bench5 images remain in Docker after the 2026-11-13 cleanup; the
intermediate `pre1`/`pre2` toolchain images were removed.

## Build

The canonical path starts from the task's public image:

```bash
cd /path/to/litecode
bash eval/harbor/prebake/build.sh
```

`build.sh` builds `Dockerfile.toolchain` (apt + Node 22 + npm CLIs) and then
`Dockerfile.harness` (grok + verification of all five binaries), one image per
task. It writes `images.txt`.

If you already have a verified toolchain image per task, use the fast path:

```bash
EVAL_PREBAKE_BASE_PROVE_PLUS_COMM=litecode-eval/prove-plus-comm:pre2 \
EVAL_PREBAKE_BASE_POLYGLOT_C_PY=litecode-eval/polyglot-c-py:pre1 \
EVAL_PREBAKE_BASE_PASSWORD_RECOVERY=litecode-eval/password-recovery:pre1 \
  bash eval/harbor/prebake/build.sh
```

The eval dataset copy (`~/eval/datasets/bench5`) is prepared by
`run_matrix.py`; it rewrites only `environment.docker_image` and records the
original value in `manifest.json`.

## Why prebake

Harbor's stock installed agents install their CLI inside every trial. In the
previous round that meant apt + nvm + npm repeatedly, opencode once took over
four minutes, and codex hit setup timeouts. The prebaked adapters in
`eval/harbor/prebaked_agents.py` verify the binary and skip the installer; the
model request path is unchanged.
## Base images

The toolchain currently supports two families:

* Ubuntu 24.04 task images (the original three bench5 tasks);
* Debian 12 bookworm task images (`cancel-async-tasks`).

`Dockerfile.toolchain` rewrites apt sources only when `/etc/os-release` says
Ubuntu; Debian keeps its default bookworm sources. Node/npm are installed only
through nvm (Node 22), so the distro's older Node package is not a dependency.
It also installs bash/coreutils/procps/ripgrep/xz-utils for the harness runtimes.

Other bases (Alpine/musl, non-root, Windows containers) need a separate adapter;
`run_matrix.py --preflight-only` rejects them rather than producing missing
trials.
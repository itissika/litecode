# WP-155 re-run kit (archived copies; originals were under .tmp/wp154)
- Tools live here; working data (corpora `big`/`small`, index seeds, `raw/`) stays in `/workspace/litecode/.tmp/wp154` (the scripts `cd` there).
- `w3.sh` (one-time large-repo first build) · `w4.sh` (full W4) · `w4b.sh` (idempotent resume: skips runs with a record; guard + progress line per run) · `w4_report.py` (tables from raw/w4)
- `probe/run.mjs` (node, no deps) · `mincore.py` + `evict.md` (Session's page-cache eviction tool and its notes; referenced by `--true-cold 1`)
- `harness/` (Cargo.toml, Cargo.lock, src/main.rs, run.sh). The binary was deleted; rebuilding is a heavy task and needs backend_mid's release:
  `cd .litecode/ops/artifacts/WP-155/harness && CARGO_TARGET_DIR=/workspace/litecode/target flock /workspace/litecode/.tmp/cargo-target.lock cargo build --release -j2`
- Guards everywhere: MemAvailable ≥ 6 GiB and no cargo/rustc before each run; taskset 0-3; 3.5 GiB RSS watchdog; isolated HOME per run.

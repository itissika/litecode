# WP-155 — back-end workspace-open path latency (IDE), W3 + W4 results

Binary under test: `target/release/litecode` @ HEAD 3ddc6433 (tree dirty only in ops files). Read-only study: src was never touched.
Measured 2026-10-14 23:46 – 2026-10-15 00:38 HKT (W4). Guard before every run: MemAvailable ≥ 6 GiB and no cargo/rustc running. All runs used taskset 0-3, a 3.5 GiB RSS watchdog, an isolated HOME / LITECODE_HOME per run, and the production embedder (granite-97m ORT q8q4).

## 1. Definitions
- **T0** = the moment the probe sends the WS upgrade request. Every time below is in ms relative to T0 on a monotonic clock. Negative values happen before T0.
- **spawn / READY:** spawn is when the process starts. READY is the `LITECODE_READY` stdout line. *spawn→READY* is the process-start segment, reported separately so the shell can join it to its own timing.
- **Input-box proxies (front-end ruling, R3):**
  - **new profile:** hello, then the `session/new` reply. That reply time is `t_new` (= t_int).
  - **restore profile:** the `session/list` reply, `t_list`. The `session/subscribe` of the newest session is recorded when one exists.
  - Neither proxy waits for SessionSnapshot.
- **Profile allocation:**
  - Cold tiers are a first open in a fresh HOME/workspace state, so they use `new`.
  - Hot tiers (R2, same serve process) do their first connect with `new`. The reconnect (hot_reconnect) or the reload (hot_reload, `GET /` and then a new WS) then uses `restore`.
  - Both proxies are therefore recorded for every hot run. Hot-segment times are relative to the reconnect's own T0 (for reload, T0 = the `GET /` start).
- **cs Warm:** the first 50 ms poll of `/api/workspace/engines` that shows `code_search` warm.
- **Child processes:** a 10 ms /proc poll of serve's descendants, recording each one's first_seen time.
- **Peak RSS:** serve plus all its descendants, sampled every 10 ms.
- **Tiers:**
  - `cold_noindex`: `<repo>/.litecode` is deleted first.
  - `cold_index`: `.litecode/index` is a fresh copy of the seed (R4).
  - `_tc`: the same, plus `--true-cold 1`, which evicts the page cache right before spawn (§4).
  - `hot_reconnect` / `hot_reload`: R2.
- **Large-repo `cold_noindex`:** measured only up to READY, the input-box proxy, and Warm, then shut down with stdin EOF. The probe did not wait for the build.

## 2. Key medians (n=3 per tier; ms rel. T0; RSS in MB)
| repo | tier | spawn→READY | READY | WS open | hello | t_new | cs Warm | peak RSS MB | hot: index.html | hot: WS open | hot: hello | hot: t_list |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| small | cold_noindex | 84.5 | -3.5 | 1.2 | 7.0 | 55.6 | 1296.7 | 564.7 | – | – | – | – |
| small | cold_noindex_tc | 103.4 | -4.2 | 7.8 | 7.9 | 59.1 | 1389.5 | 562.6 | – | – | – | – |
| small | cold_index | 83.1 | -3.4 | 6.8 | 8.0 | 62.2 | 1445.2 | 556.6 | – | – | – | – |
| small | cold_index_tc | 96.8 | -2.4 | 1.0 | 6.2 | 51.7 | 1510.2 | 553.6 | – | – | – | – |
| small | hot_reconnect | 78.1 | -5.1 | 8.1 | 8.1 | 58.5 | 1479.5 | 555.9 | – | 0.8 | 1.1 | 3.0 |
| small | hot_reload | 68.0 | -8.0 | 8.5 | 8.5 | 57.1 | 1463.0 | 556.0 | 1.6 | 3.3 | 9.1 | 11.5 |
| big | cold_noindex | 525.5 | -2.3 | 0.9 | 5.5 | 50.7 | 1228.7 | 597.0 | – | – | – | – |
| big | cold_noindex_tc | 524.5 | -3.7 | 1.5 | 5.5 | 53.9 | 1280.7 | 573.3 | – | – | – | – |
| big | cold_index | 420.1 | -2.7 | 0.8 | 6.2 | 58.2 | 2591.7 | 578.2 | – | – | – | – |
| big | cold_index_tc | 588.0 | -3.0 | 5.6 | 6.2 | 56.7 | 2495.4 | 567.9 | – | – | – | – |
| big | hot_reconnect | 397.4 | -4.4 | 1.3 | 6.5 | 59.3 | 2563.2 | 575.0 | – | 1.0 | 1.3 | 3.5 |
| big | hot_reload | 457.3 | -6.8 | 9.1 | 9.2 | 67.1 | 2459.5 | 582.2 | 7.5 | 10.7 | 10.9 | 19.5 |
Readings:
- **The input-box proxy is flat across every tier and both repos:** t_new is 50–67 ms. Of that, about 45–50 ms is the `session/new` round trip itself; hello arrives 1–9 ms after T0. Index state, repo size and true-cold all leave it unchanged.
- **Restore (hot) is fast:** t_list is 3.0–3.5 ms for reconnect and 11.5–19.5 ms for reload, with the reload's `GET /` (stub) at 1.6–7.5 ms.
- **Large repo, spawn→READY is 400–590 ms; small repo is 68–103 ms.** The harness shows the serve_order steps themselves sum to about 28 ms on the large repo. The rest falls before READY, and the evidence points to first-run work under a fresh HOME:
  - `git --git-dir <HOME>/.litecode/snapshots/big_…` child processes appear at about −430 to −350 ms. They do not appear on the small repo.
  - Harness run 1 on the large repo, the first in `home-harness`, spent 373 ms in `load_runtime_bundle` (`src/cli/cli.rs:93`). Runs 2 and 3 took 16–17 ms there.
  - This is pre-READY, so it sits on the shell's start segment. It is outside the T0→t_int window.
- **cs Warm is background work** and is not on the t_int path. It takes 1.2–1.5 s on the small repo and on the large repo without an index (the worker spawn plus ORT load). It takes 2.4–2.6 s on the large repo with an index, which adds about 1.2 s of index load (usearch, BM25, stamps).
- **rust-analyzer:** only a `rust-analyzer --version` availability probe runs, at about +370 to +565 ms (after t_int). No language server is spawned and no cargo process runs. *(Corrected 00:5x: an earlier note called the +495 ms process a `cargo` child. It was the rustup proxy `.litecode/ops/.toolchain/cargo/bin/rust-analyzer --version`; see §13.)*
- **Peak RSS during open is 550–625 MB** for serve plus the worker and RA, far below the 3.5 GiB watchdog. No watchdog fired, and no processes were left behind.

## 3. Per-tier raw values and medians
Columns, all relative to T0 (the first-batch requests are listed here):
- **First batch:** `session/new sent` is when the request was sent and `t_new (reply)` is when its reply arrived.
- **Hot runs:** the `H:` columns are the reconnect or reload, using the restore profile. `H: session/list sent` is the request and `H: t_list` its reply. `H: subscribe` is empty because the list came back empty (§6.2).
- **Child processes:** first_seen, all runs listed.

### small / cold_noindex  (n=3)

| run | READY(spawn→) | spawn | READY | WS open | hello | session/new sent | t_new (reply) | cs Warm | peak RSS MB |
|---|---|---|---|---|---|---|---|---|---|
| r1 | 84.5 | -87.0 | -2.5 | 1.1 | 6.3 | 6.7 | 55.6 | 1493.9 | 561.1 |
| r2 | 98.8 | -106.7 | -7.9 | 1.2 | 9.5 | 9.9 | 52.0 | 1289.2 | 564.7 |
| r3 | 84.1 | -87.6 | -3.5 | 6.9 | 7.0 | 7.2 | 57.9 | 1296.7 | 580.2 |
| **median** | 84.5 | -87.6 | -3.5 | 1.2 | 7.0 | 7.2 | 55.6 | 1296.7 | 564.7 |

child processes (first_seen ms rel. T0, all runs): (no args): -26, -52.9; cs-worker: -2.9, -21.5, -4.9; npm: -2.8, -21.4, -4.8; rust-analyzer: 491.3, 387.0, 399.1

### small / cold_noindex_tc  (n=3)

| run | READY(spawn→) | spawn | READY | WS open | hello | session/new sent | t_new (reply) | cs Warm | peak RSS MB |
|---|---|---|---|---|---|---|---|---|---|
| r1 | 161.9 | -175.4 | -13.5 | 1.4 | 7.8 | 8.1 | 59.4 | 1389.5 | 562.6 |
| r2 | 98.0 | -102.2 | -4.2 | 7.8 | 7.9 | 8.3 | 56.4 | 1352.6 | 556.7 |
| r3 | 103.4 | -107.4 | -4.0 | 8.2 | 8.3 | 8.6 | 59.1 | 1398.6 | 565.2 |
| **median** | 103.4 | -107.4 | -4.2 | 7.8 | 7.9 | 8.3 | 59.1 | 1389.5 | 562.6 |

child processes (first_seen ms rel. T0, all runs): (no args): -75.0, -57.6, -37.9; cs-worker: -32.3, -4.6, -13.4; npm: -32.2, -4.5, -13.3; rust-analyzer: 448.1, 427.6, 404.1

mincore: 
- r1: before 31437/39197 pages resident over 142 files → after 1687 pages; still resident: libstdc++.so.6 610, libgcc_s.so.1 45, libm.so.6 239, libmvec.so.1 249, libc.so.6 488, ld-linux-x86-64.so.2 56; verified=False own_files_evicted=True
- r2: before 15677/39197 pages resident over 142 files → after 1687 pages; still resident: libstdc++.so.6 610, libgcc_s.so.1 45, libm.so.6 239, libmvec.so.1 249, libc.so.6 488, ld-linux-x86-64.so.2 56; verified=False own_files_evicted=True
- r3: before 15677/39197 pages resident over 142 files → after 1687 pages; still resident: libstdc++.so.6 610, libgcc_s.so.1 45, libm.so.6 239, libmvec.so.1 249, libc.so.6 488, ld-linux-x86-64.so.2 56; verified=False own_files_evicted=True

### small / cold_index  (n=3)

| run | READY(spawn→) | spawn | READY | WS open | hello | session/new sent | t_new (reply) | cs Warm | peak RSS MB |
|---|---|---|---|---|---|---|---|---|---|
| r1 | 133.6 | -137.7 | -4.1 | 5.9 | 7.3 | 7.5 | 51.6 | 1627.5 | 553.8 |
| r2 | 81.4 | -84.8 | -3.4 | 6.8 | 10.4 | 10.6 | 90.4 | 1358.7 | 557.3 |
| r3 | 83.1 | -85.8 | -2.7 | 8.0 | 8.0 | 8.2 | 62.2 | 1445.2 | 556.6 |
| **median** | 83.1 | -85.8 | -3.4 | 6.8 | 8.0 | 8.2 | 62.2 | 1445.2 | 556.6 |

child processes (first_seen ms rel. T0, all runs): (no args): -53.1, -34.1, -43.4, -23.3, -42.5, -33.2; cs-worker: -4.6, -13.5, -10.7; npm: -4.5, -13.4, -10.6; rust-analyzer: 566.2, 372.7, 365.4

### small / cold_index_tc  (n=3)

| run | READY(spawn→) | spawn | READY | WS open | hello | session/new sent | t_new (reply) | cs Warm | peak RSS MB |
|---|---|---|---|---|---|---|---|---|---|
| r1 | 96.8 | -99.2 | -2.4 | 0.8 | 6.1 | 6.3 | 50.9 | 1379.3 | 570.0 |
| r2 | 96.9 | -107.8 | -10.9 | 7.0 | 7.1 | 7.3 | 51.7 | 1562.0 | 553.6 |
| r3 | 92.7 | -95.0 | -2.3 | 1.0 | 6.2 | 6.4 | 89.5 | 1510.2 | 553.5 |
| **median** | 96.8 | -99.2 | -2.4 | 1.0 | 6.2 | 6.4 | 51.7 | 1510.2 | 553.6 |

child processes (first_seen ms rel. T0, all runs): cs-worker: -2.7, -15.1, -2.6; npm: -2.6, -15.0, -2.6; rust-analyzer: 382.9, 478.4, 386.9; (no args): -34.3

mincore: 
- r1: before 16288/39695 pages resident over 170 files → after 1687 pages; still resident: libstdc++.so.6 610, libgcc_s.so.1 45, libm.so.6 239, libmvec.so.1 249, libc.so.6 488, ld-linux-x86-64.so.2 56; verified=False own_files_evicted=True
- r2: before 16290/39695 pages resident over 170 files → after 1687 pages; still resident: libstdc++.so.6 610, libgcc_s.so.1 45, libm.so.6 239, libmvec.so.1 249, libc.so.6 488, ld-linux-x86-64.so.2 56; verified=False own_files_evicted=True
- r3: before 16290/39695 pages resident over 170 files → after 1687 pages; still resident: libstdc++.so.6 610, libgcc_s.so.1 45, libm.so.6 239, libmvec.so.1 249, libc.so.6 488, ld-linux-x86-64.so.2 56; verified=False own_files_evicted=True

### small / hot_reconnect  (n=3)

| run | READY(spawn→) | spawn | READY | WS open | hello | session/new sent | t_new (reply) | cs Warm | peak RSS MB | H: index.html | H: WS open | H: hello | H: session/list sent | H: t_list | H: subscribe |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| r1 | 87.0 | -96.9 | -9.9 | 1.1 | 7.8 | 8.0 | 59.7 | 1751.9 | 551.6 | – | 0.5 | 1.1 | 1.2 | 2.6 | – |
| r2 | 78.1 | -80.6 | -2.4 | 8.1 | 8.1 | 8.4 | 58.5 | 1479.5 | 557.5 | – | 0.9 | 1.3 | 1.4 | 3.0 | – |
| r3 | 73.3 | -78.4 | -5.1 | 8.2 | 8.2 | 8.6 | 57.5 | 1337.3 | 555.9 | – | 0.8 | 1.1 | 1.1 | 3.2 | – |
| **median** | 78.1 | -80.6 | -5.1 | 8.1 | 8.1 | 8.4 | 58.5 | 1479.5 | 555.9 | – | 0.8 | 1.1 | 1.2 | 3.0 | – |

child processes (first_seen ms rel. T0, all runs): (no args): -42.0, -21.3, -32.6, -36; cs-worker: -2.7, -2.9, -5.5; npm: -2.7, -2.9, -5.5; rust-analyzer: 401.9, 367.1, 371.7

### small / hot_reload  (n=3)

| run | READY(spawn→) | spawn | READY | WS open | hello | session/new sent | t_new (reply) | cs Warm | peak RSS MB | H: index.html | H: WS open | H: hello | H: session/list sent | H: t_list | H: subscribe |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| r1 | 68.0 | -77.8 | -9.8 | 8.5 | 8.5 | 8.8 | 67.8 | 1463.0 | 573.7 | 2.9 | 4.2 | 9.1 | 9.2 | 11.5 | – |
| r2 | 67.2 | -75.2 | -8.0 | 1.3 | 1.5 | 1.6 | 48.5 | 1415.8 | 554.7 | 1.6 | 2.7 | 3.2 | 3.3 | 10 | – |
| r3 | 74.3 | -77.1 | -2.8 | 8.9 | 9.0 | 9.2 | 57.1 | 1468.6 | 556.0 | 1.6 | 3.3 | 10.4 | 10.5 | 60.2 | – |
| **median** | 68.0 | -77.1 | -8.0 | 8.5 | 8.5 | 8.8 | 57.1 | 1463.0 | 556.0 | 1.6 | 3.3 | 9.1 | 9.2 | 11.5 | – |

child processes (first_seen ms rel. T0, all runs): cs-worker: -2.1, -2.2, -3.3; npm: -2.1, -2.2, -3.2; rust-analyzer: 483.4, 362.3; (no args): -22.3, 465.4

### big / cold_noindex  (n=3)

| run | READY(spawn→) | spawn | READY | WS open | hello | session/new sent | t_new (reply) | cs Warm | peak RSS MB |
|---|---|---|---|---|---|---|---|---|---|
| r1 | 525.5 | -527.9 | -2.3 | 0.9 | 6.4 | 6.7 | 49.7 | 1222.0 | 574.2 |
| r2 | 475.7 | -478.1 | -2.5 | 0.9 | 1.1 | 1.4 | 50.7 | 1432.8 | 597.0 |
| r3 | 609.2 | -611.4 | -2.2 | 5.4 | 5.5 | 5.7 | 59.3 | 1228.7 | 608.7 |
| **median** | 525.5 | -527.9 | -2.3 | 0.9 | 5.5 | 5.7 | 50.7 | 1228.7 | 597.0 |

child processes (first_seen ms rel. T0, all runs): (no args): -485.7, -465.3, -452.6, -402.5, -550.4, -540.2; git: -434.5, -34.2, -382.2, -60.4, -518.4, -72.2; cs-worker: -4.6, -11.3, -16.4; npm: -4.5, -3.0, -16.4; rust-analyzer: 434.0, 599.7, 564.7

### big / cold_noindex_tc  (n=3)

| run | READY(spawn→) | spawn | READY | WS open | hello | session/new sent | t_new (reply) | cs Warm | peak RSS MB |
|---|---|---|---|---|---|---|---|---|---|
| r1 | 654.2 | -663.0 | -8.7 | 1.5 | 5.5 | 6.0 | 53.9 | 1324.2 | 604.4 |
| r2 | 524.5 | -528.0 | -3.6 | 0.9 | 2.3 | 2.7 | 52.1 | 1280.7 | 570.0 |
| r3 | 508.1 | -511.7 | -3.7 | 7.2 | 7.3 | 7.7 | 59.1 | 1275.8 | 573.3 |
| **median** | 524.5 | -528.0 | -3.7 | 1.5 | 5.5 | 6.0 | 53.9 | 1280.7 | 573.3 |

child processes (first_seen ms rel. T0, all runs): (no args): -499.9, -454.6, -445.7; git: -481.8, -74.1, -426.2, -55.1, -417.0, -56.5; cs-worker: -32.9, -13, -15.8; npm: -32.8, -12.9, -15.8; rust-analyzer: 427.3, 434.4, 382.6

mincore: 
- r1: before 20225/65344 pages resident over 1269 files → after 1687 pages; still resident: libstdc++.so.6 610, libgcc_s.so.1 45, libm.so.6 239, libmvec.so.1 249, libc.so.6 488, ld-linux-x86-64.so.2 56; verified=False own_files_evicted=True
- r2: before 20061/65344 pages resident over 1269 files → after 1687 pages; still resident: libstdc++.so.6 610, libgcc_s.so.1 45, libm.so.6 239, libmvec.so.1 249, libc.so.6 488, ld-linux-x86-64.so.2 56; verified=False own_files_evicted=True
- r3: before 20061/65344 pages resident over 1269 files → after 1687 pages; still resident: libstdc++.so.6 610, libgcc_s.so.1 45, libm.so.6 239, libmvec.so.1 249, libc.so.6 488, ld-linux-x86-64.so.2 56; verified=False own_files_evicted=True

### big / cold_index  (n=3)

| run | READY(spawn→) | spawn | READY | WS open | hello | session/new sent | t_new (reply) | cs Warm | peak RSS MB |
|---|---|---|---|---|---|---|---|---|---|
| r1 | 420.1 | -422.8 | -2.7 | 8.0 | 8.1 | 8.6 | 68.2 | 2394.8 | 574.9 |
| r2 | 410.8 | -420.2 | -9.5 | 0.8 | 0.9 | 1.2 | 57.5 | 2691.1 | 580.3 |
| r3 | 506.5 | -509.1 | -2.6 | 0.8 | 6.2 | 6.5 | 58.2 | 2591.7 | 578.2 |
| **median** | 420.1 | -422.8 | -2.7 | 0.8 | 6.2 | 6.5 | 58.2 | 2591.7 | 578.2 |

child processes (first_seen ms rel. T0, all runs): (no args): -367.9, -356.6, -466.2; git: -348.1, -44.3, -347.5, -44.7, -425.9, -54.1; cs-worker: -13.1, -14.3, -23.9; npm: -13.1, -14.3, -12.1; rust-analyzer: 439.9, 411.7, 402.2

### big / cold_index_tc  (n=3)

| run | READY(spawn→) | spawn | READY | WS open | hello | session/new sent | t_new (reply) | cs Warm | peak RSS MB |
|---|---|---|---|---|---|---|---|---|---|
| r1 | 769.8 | -774.4 | -4.6 | 6.4 | 6.4 | 6.7 | 60.6 | 2415.6 | 574.2 |
| r2 | 588.0 | -590.9 | -3.0 | 1.2 | 6.2 | 6.5 | 51.4 | 2495.4 | 567.1 |
| r3 | 508.2 | -510.5 | -2.3 | 5.6 | 5.6 | 6.0 | 56.7 | 3261.2 | 567.9 |
| **median** | 588.0 | -590.9 | -3.0 | 5.6 | 6.2 | 6.5 | 56.7 | 2495.4 | 567.9 |

child processes (first_seen ms rel. T0, all runs): (no args): -703, -692.6, -515.8, -498.1, -489.6, -435.6; git: -683.2, -228.9, -468.1, -53.9, -424.7, -34.9; cs-worker: -14.8, -13.8, -14.7; npm: -14.7, -13.7, -4.2; rust-analyzer: 397.6, 497.4, 524.7

mincore: 
- r1: before 25452/70620 pages resident over 1297 files → after 1687 pages; still resident: libstdc++.so.6 610, libgcc_s.so.1 45, libm.so.6 239, libmvec.so.1 249, libc.so.6 488, ld-linux-x86-64.so.2 56; verified=False own_files_evicted=True
- r2: before 25452/70620 pages resident over 1297 files → after 1687 pages; still resident: libstdc++.so.6 610, libgcc_s.so.1 45, libm.so.6 239, libmvec.so.1 249, libc.so.6 488, ld-linux-x86-64.so.2 56; verified=False own_files_evicted=True
- r3: before 25452/70620 pages resident over 1297 files → after 1687 pages; still resident: libstdc++.so.6 610, libgcc_s.so.1 45, libm.so.6 239, libmvec.so.1 249, libc.so.6 488, ld-linux-x86-64.so.2 56; verified=False own_files_evicted=True

### big / hot_reconnect  (n=3)

| run | READY(spawn→) | spawn | READY | WS open | hello | session/new sent | t_new (reply) | cs Warm | peak RSS MB | H: index.html | H: WS open | H: hello | H: session/list sent | H: t_list | H: subscribe |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| r1 | 389.3 | -391.5 | -2.2 | 0.9 | 6.5 | 6.7 | 59.3 | 2539.0 | 574.1 | – | 1.0 | 1.1 | 1.2 | 2.8 | – |
| r2 | 420.4 | -431.8 | -11.4 | 1.3 | 11.2 | 11.8 | 71.0 | 2563.2 | 586.0 | – | 0.8 | 1.3 | 1.4 | 3.5 | – |
| r3 | 397.4 | -401.8 | -4.4 | 6.2 | 6.3 | 6.5 | 49.5 | 2736.9 | 575.0 | – | 1.1 | 1.3 | 1.4 | 8.9 | – |
| **median** | 397.4 | -401.8 | -4.4 | 1.3 | 6.5 | 6.7 | 59.3 | 2563.2 | 575.0 | – | 1.0 | 1.3 | 1.4 | 3.5 | – |

child processes (first_seen ms rel. T0, all runs): (no args): -357.2, -387.0, -378.4, -367.9, -355.8; git: -327.7, -32.9, -357.6, -54.9, -323.0, -35.8; cs-worker: -4.2, -13.4, -15.0; npm: -4.1, -13.4, -4.9; rust-analyzer: 402.2, 446.7, 383.7

### big / hot_reload  (n=3)

| run | READY(spawn→) | spawn | READY | WS open | hello | session/new sent | t_new (reply) | cs Warm | peak RSS MB | H: index.html | H: WS open | H: hello | H: session/list sent | H: t_list | H: subscribe |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| r1 | 457.3 | -464.1 | -6.8 | 2.7 | 2.9 | 3.0 | 59.4 | 2385.2 | 582.2 | 7.5 | 8.9 | 9.2 | 9.3 | 16.6 | – |
| r2 | 439.0 | -443.5 | -4.5 | 11.4 | 11.4 | 11.8 | 70.2 | 2459.5 | 571.1 | 10.1 | 11.8 | 19.4 | 19.6 | 60.1 | – |
| r3 | 680.9 | -693.5 | -12.7 | 9.1 | 9.2 | 9.6 | 67.1 | 2511.3 | 625.0 | 1.3 | 10.7 | 10.9 | 11 | 19.5 | – |
| **median** | 457.3 | -464.1 | -6.8 | 9.1 | 9.2 | 9.6 | 67.1 | 2459.5 | 582.2 | 7.5 | 10.7 | 10.9 | 11 | 19.5 | – |

child processes (first_seen ms rel. T0, all runs): (no args): -410.8, -388.4, -378.5, -650.8, -631.2; git: -389.4, -41.8, -368.2, -45.2, -621.5, -53.3; cs-worker: -11.9, -17.9, -23.4; npm: -11.9, -17.8, -23.3; rust-analyzer: 494.8, 425.6, 399.5
## 4. True cold (page cache) and mincore readings
- Eviction uses `mincore.py` from Session's evict.md: fsync plus `POSIX_FADV_DONTNEED`, with a mincore check before and after. It runs right before spawn, after all corpus, seed and engines.json writes, so nothing reads the files in between.
- **Targets:** the binary under test, its ldd libraries, the model directory, the repo (including the copied index), the web stub, and the run's HOME.
- **Result on every one of the 12 runs:** all of the run's own files dropped to 0 resident pages (`own_files_evicted=True`). What stays resident is the 6 system libraries, 1,687 pages in total:
  - libstdc++.so.6: 610 pages
  - libc.so.6: 488 pages
  - libmvec.so.1: 249 pages
  - libm.so.6: 239 pages
  - ld-linux-x86-64.so.2: 56 pages
  - libgcc_s.so.1: 45 pages
- **These libraries cannot be evicted**, because every running process maps them. That is why the `verified` flag reads False. The tool's strict check includes the libraries; the own-files check passes.
- **Since the true-cold tiers exist, the cold tiers are no longer labelled "lower bound"** (this replaces R5). The only residual warm content is the 6 system libraries above.
- **Measured true-cold cost:**
  - Small repo: spawn→READY rises by about +12 to +19 ms (median 103/97 vs 85/83 ms).
  - Large repo, with an index: about +170 ms (588 vs 420 ms). Without an index: about 0 (525 vs 525 ms).
  - t_new and cs Warm show no consistent shift; the spread is within run-to-run noise.

## 5. Harness step breakdown (R1-A; `.litecode/ops/artifacts/WP-155/harness`, same public calls in serve::run order)
`serve_order` reproduces `serve::run` up to `TcpListener::bind`, then waits for code_search Warm. `decompose` times the public pieces of `ServeState::new` on their own. The harness re-uses `home-harness` across runs, so run 1 is that HOME's first run.

### harness small / serve_order (n=3)

| step | source | r1 | r2 | r3 | median ms |
|---|---|---|---|---|---|
| load_runtime_bundle | `src/cli/cli.rs:93` | 13.65 | 12.32 | 10.85 | 12.32 |
| resolve_web_dist | `src/serve/run.rs:70` | 0.01 | 0.01 | 0.01 | 0.01 |
| sweep_orphan_lsp_processes | `src/serve/run.rs:75 → src/proc_lifetime.rs:48` | 2.08 | 2.16 | 2.52 | 2.16 |
| EngineManager::reconcile | `src/serve/run.rs:84` | 0.37 | 0.83 | 0.34 | 0.37 |
| WorkspaceEngines::reconcile (spawns warmup threads) | `src/serve/run.rs:85 → src/engines/mod.rs:773/1105` | 0.09 | 0.11 | 0.11 | 0.11 |
| path_index.attach | `src/serve/run.rs:86 → src/engines/path_index/mod.rs:83` | 0.04 | 0.04 | 0.05 | 0.04 |
| ServeState::new | `src/serve/run.rs:91 → src/serve/state.rs:37` | 3.27 | 3.27 | 3.13 | 3.27 |
| tokio runtime build | `src/serve/run.rs:110` | 0.16 | 3.56 | 5.51 | 3.56 |
| restart_watcher (awaited before bind) | `src/serve/router.rs:112 → src/workspace/watcher.rs:270` | 4.02 | 4.43 | 0.99 | 4.02 |
| TcpListener::bind | `src/serve/router.rs:253` | 0.05 | 0.04 | 0.03 | 0.04 |
| _sum to bind end_ | | 24.09 | 27.04 | 23.89 | 24.09 |
| _code_search Warm (bg)_ | `src/engines/code_search_engine.rs:374` | 1421 | 1571 | 1469 | 1469 |

### harness small / decompose (n=3)

| step | source | r1 | r2 | r3 | median ms |
|---|---|---|---|---|---|
| load_runtime_bundle | `src/cli/cli.rs:93` | 10.34 | 11.50 | 10.54 | 10.54 |
| WorkspaceWriteLease::acquire | `src/serve/state.rs:~85` | 0.02 | 0.02 | 0.02 | 0.02 |
| SessionData::open (sessions.db) | `src/serve/state.rs:90` | 1.77 | 1.76 | 2.30 | 1.77 |
| list_session_ids | `src/serve/state.rs:99` | 0.33 | 0.38 | 0.34 | 0.34 |
| maintain_snapshots | `src/serve/state.rs:97` | 0.15 | 0.15 | 0.17 | 0.15 |
| WorkspaceService::new | `src/serve/state.rs:111 → src/workspace/service.rs:97` | 0.01 | 0.01 | 0.02 | 0.01 |
| TerminalHub::new | `src/serve/state.rs:112` | 0.00 | 0.00 | 0.00 | 0.00 |

session_count: [7, 7, 7]

### harness big / serve_order (n=3)

| step | source | r1 | r2 | r3 | median ms |
|---|---|---|---|---|---|
| load_runtime_bundle | `src/cli/cli.rs:93` | 373.47 | 15.87 | 16.92 | 16.92 |
| resolve_web_dist | `src/serve/run.rs:70` | 0.01 | 0.01 | 0.01 | 0.01 |
| sweep_orphan_lsp_processes | `src/serve/run.rs:75 → src/proc_lifetime.rs:48` | 2.07 | 2.17 | 2.40 | 2.17 |
| EngineManager::reconcile | `src/serve/run.rs:84` | 0.43 | 0.35 | 0.61 | 0.43 |
| WorkspaceEngines::reconcile (spawns warmup threads) | `src/serve/run.rs:85 → src/engines/mod.rs:773/1105` | 0.11 | 0.11 | 0.11 | 0.11 |
| path_index.attach | `src/serve/run.rs:86 → src/engines/path_index/mod.rs:83` | 0.08 | 0.03 | 0.03 | 0.03 |
| ServeState::new | `src/serve/run.rs:91 → src/serve/state.rs:37` | 5.32 | 3.53 | 2.67 | 3.53 |
| tokio runtime build | `src/serve/run.rs:110` | 0.14 | 0.16 | 0.13 | 0.14 |
| restart_watcher (awaited before bind) | `src/serve/router.rs:112 → src/workspace/watcher.rs:270` | 5.01 | 4.62 | 5.33 | 5.01 |
| TcpListener::bind | `src/serve/router.rs:253` | 0.03 | 0.04 | 0.13 | 0.04 |
| _sum to bind end_ | | 386.93 | 27.16 | 28.57 | 28.57 |
| _code_search Warm (bg)_ | `src/engines/code_search_engine.rs:374` | 2706 | 2336 | 2312 | 2336 |

### harness big / decompose (n=3)

| step | source | r1 | r2 | r3 | median ms |
|---|---|---|---|---|---|
| load_runtime_bundle | `src/cli/cli.rs:93` | 18.56 | 18.71 | 16.25 | 18.56 |
| WorkspaceWriteLease::acquire | `src/serve/state.rs:~85` | 0.02 | 0.02 | 0.02 | 0.02 |
| SessionData::open (sessions.db) | `src/serve/state.rs:90` | 1.82 | 1.97 | 2.03 | 1.97 |
| list_session_ids | `src/serve/state.rs:99` | 0.43 | 0.32 | 0.46 | 0.43 |
| maintain_snapshots | `src/serve/state.rs:97` | 0.20 | 0.15 | 0.20 | 0.20 |
| WorkspaceService::new | `src/serve/state.rs:111 → src/workspace/service.rs:97` | 0.02 | 0.01 | 0.02 | 0.02 |
| TerminalHub::new | `src/serve/state.rs:112` | 0.00 | 0.00 | 0.00 | 0.00 |

session_count: [7, 7, 7]
- **Before bind, `load_runtime_bundle` dominates** (10–19 ms warm, 373 ms on the first run against the large repo, §2). Next come `restart_watcher` (1–5 ms, awaited), `ServeState::new` (3–5 ms) and `sweep_orphan_lsp_processes` (2 ms). Everything else is under 1 ms.
- **The ServeState pieces are all small:** sessions.db open about 2 ms, list ids about 0.4 ms, maintain_snapshots about 0.2 ms.
- **No step inside serve::run is a material share of t_int.** The open path's serial cost before the input box is the `session/new` round trip of about 45 ms (Session's domain) plus the first-HOME `load_runtime_bundle` cost before READY on the large repo.

## 6. Limits and anomalies
1. **Stub index.html:** `LITECODE_WEB_DIST` points to `.tmp/wp154/webstub` (a single small index.html). So the reload's `GET /` time does not include real asset loading or JS execution. The front end measures resource loading and JS execution.
2. **The restore proxy saw an empty session list:**
   - The session created by `session/new` on the first connect is not returned by `session/list` (`session_count=0`), even though sessions.db holds it: harness `list_session_ids` sees 7 accumulated IDs.
   - Empty sessions appear to be filtered out of the list. So `t_list` was measured on an empty list and `session/subscribe` was never sent.
   - Populating a session would need a model turn, which I did not do.
   - If Session or the front end need the populated-list case, it needs a seeded sessions.db.
3. **One guard refusal:** small hot_reload r3 was refused at 00:09:51, because MemAvailable dropped below 6 GiB between the script's guard and the probe's own check. It was re-run at 00:37:29, so n=3 holds. Guard waits also occurred at 23:57 and from 00:09:51 (about 12 min, during other agents' builds).
4. **Harness hang (fixed, no data lost):**
   - The first harness attempt never reached Warm, because the harness binary spawned *itself* as `code-search-worker`. The worker client uses `CARGO_BIN_EXE_litecode`, else `current_exe` (`src/engines/code_search_ipc/client.rs:27-30`).
   - I killed that attempt; it had produced no records.
   - `run.sh` now sets `CARGO_BIN_EXE_litecode` to the release litecode binary, and all 12 harness runs then completed.
5. **The true-cold `verified` flag is False** only because of the 6 system libraries (§4).
6. **The W1 binary build's peak RSS is unknown** (the bash watchdog crashed; see the progress log). The harness build peaked at 2.73 GB.

## 7. W3 — large-repo first build (once, R4)
- **Timing:**
  - Start 23:24:10 HKT (MemAvailable 6,319,804 kB).
  - `chunks.total` = 6257, first seen at 23:24:41.
  - Index done at T0 + 1102.7 s (about 23:42:3x).
  - Probe exit 23:44:56.
- **Embedding rate:** about 5.8 chunks/s (6257 chunks over about 1,070 s). The earlier subset reference was 7.2 chunks/s.
- **Peak RSS:** 2,869,260 kB (serve plus worker), with no watchdog. The input box during the build was t_int = 68.6 ms and cs Warm 1,423.5 ms.
- **Seed `index-seed-big/index`:** 6257 lines in chunks.jsonl = meta.indexed_chunks = job total, next_id 6258, 1,028 files, pipeline v7, dim 384.
- **Small seed `index-seed-small/index`** (built at the start of W4, not a measurement): 518 chunks.

## 8. Re-run commands (kit archived in `.litecode/ops/artifacts/WP-155/`, see its README.md)
- **Working data** (corpora `big`/`small`, index seeds, `raw/`) stays in `/workspace/litecode/.tmp/wp154`. The scripts `cd` there.
- **Probe runs:** `bash .litecode/ops/artifacts/WP-155/w4b.sh`. It is idempotent: it skips runs that already have a record, guards each run (MemAvailable ≥ 6 GiB, no cargo/rustc), and writes one progress line per run. The full sequence is `w4.sh`. The one-time large-repo build is `w3.sh`.
- **Single probe run:** `node .litecode/ops/artifacts/WP-155/probe/run.mjs --bin target/release/litecode --corpus <dir> --scenario cold_noindex|cold_index|hot_reconnect|hot_reload --profile new [--hot-profile restore] [--index-seed <seed>] [--true-cold 1] --runs N --out <file.jsonl>`.
- **True cold:** uses `.litecode/ops/artifacts/WP-155/mincore.py`. Its notes are in `evict.md`.
- **Harness:** the source is in `.litecode/ops/artifacts/WP-155/harness/`. The binary was deleted; rebuilding is a heavy task and needs backend_mid's release. Then run `bash .litecode/ops/artifacts/WP-155/harness/run.sh <workspace> serve_order|decompose 3`.
- **Tables:** `python3 .litecode/ops/artifacts/WP-155/w4_report.py`.
- **Report copy and progress log:** `.litecode/ops/artifacts/WP-155/ide.md` (same content as this file) and `.litecode/ops/artifacts/WP-155/ide_progress.md`. Both were moved from `.tmp/wp154` on 2026-10-15; the kit scripts log there.
- **Raw data:** `.tmp/wp154/raw/w4/{small,big}/<tier>/r<i>/{rec.jsonl,litecode-*.log}` and `raw/w4/harness/*.json`.

## 9. Cleanup record
Done 2026-10-15 00:4x HKT:
- **Deleted, harness build artifacts in the shared `target/release`:**
  - What was removed: 1,421 entries in `deps/`, `.fingerprint/` and `build/` (about 1.3 GB), plus `wp155_open_harness` and its `.d` file.
  - What they were: everything written during the harness builds (23:01:00–23:13:40). That covers harness attempt #1, which had no lock file and resolved ort 2.0.0-rc.14 plus its feature-set dependencies, and attempt #2, which built the harness-variant litecode lib and the harness itself.
  - What was kept: the main crate's artifacts are all from 22:55–23:00:55 and were not touched. `target/release/litecode` (23:00:55) is still present. Nothing in `target/release` was written after 23:13:40, so no later build reused these units.
- **Deleted, isolated HOMEs:** 39 `home-*` directories, about 194 MB (per-run HOMEs under `raw/`, `raw/w4/`, and `home-harness`).
- **Deleted, run state inside the test repos:** `big/.litecode` and `small/.litecode`, which held the seed copies, sessions.db and logs. The probe ran on the originals, so there were no separate repo copies to delete.
- **Deleted, stray directory:** `.tmp/wp154/code-search-worker/`, which the hung harness attempt created by treating "code-search-worker" as a workspace path.
- **Kept until Session and the front end are done:** `index-seed-big` (21 MB), `index-seed-small` (2 MB), the `big` and `small` originals, `raw/` (1.3 MB: records plus copied logs), the probe, the harness *source* (no binary any more, so re-running the harness needs a rebuild), and the scripts.

---
# Conclusions (added after acceptance was rejected, 2026-10-15; no new runs; based on the W4 raw data, copied litecode.log files and the source at HEAD 3ddc6433)

## 10. Step list in three stages, with P0/P1/P2

**How to read the timing columns**
- *Logs* columns are medians per tier taken from the copied `litecode.log` timestamps, aligned to the probe clock through the READY line.
  - "cold" = `cold_noindex`, "hot" = `hot_reconnect`, written small / big.
  - Every probe run starts with a fresh HOME, so every tier is a first open for that HOME (§12).
- *Harness* columns are medians of 3 runs with a re-used HOME (only r1 was a first run), written small / big.
- "Serial" = the step blocks READY or t_int, from source order and timing (plan §3).

**P0/P1/P2 rule** (plan §5)
- P0 = needed before READY or the input box.
- P1 = needed within seconds of the first action.
- P2 = needed on demand or for cleanup.

### Stage A: spawn → READY (the shell's start segment; before T0)

| # | Step | Source | Cold median (logs) | Hot median (logs) | Harness median | Serial? | Class | Reason |
|---|---|---|---|---|---|---|---|---|
| A1 | Process start + global config: provider catalog, `migrate_once`, `repair_agent_models_on_boot`, `load_global_from`, validation | `src/cli/cli.rs:733` → `src/config/manager.rs:135-145` | inside A1–A3: spawn→log dir 69 / 508 ms | 67 / 379 ms | `load_runtime_bundle` (A1–A3) 12.3 / 16.9; first run in that HOME on big: 373 | yes | P0 | Config and providers are needed before anything else |
| A2 | `load_workspace_state`: resolve root, `init_workspace`, `ensure_workspace_identity`, `set_runtime_paths`, create snapshots dir, engines readiness, workspace defs | `src/config/workspace.rs:491-511` | (in A1 figure) | | (in A1 figure) | yes | P0 | Workspace paths and identity; sessions.db path |
| A3 | **`warm_snapshot_repo`**: `open_or_init_repo` (+ `seed_from_project_git` on init), then `cli_sync_and_write_tree` (git diff-files, ls-files --others, check-ignore, add, write-tree) under `workspace_snapshot_lock` | `src/config/workspace.rs:513` → `src/session/snapshot.rs:1003` (:140, :382, :584) | small: no git child ≥ 10 ms. Big: git children from 64–181 ms to 359–640 ms after spawn, so **≈ 300–480 ms** (§12) | big 290–300 ms | big ≈ 357 of r1's 373 ms; steady state ≈ +5 ms over small | **yes** (synchronous, before logging and bind) | **P1** | Only the first turn's `snapshot_track` needs it, not READY or the input box → §11 |
| A4 | `setup_logging` (log dir, file appender, level from db) | `src/cli/cli.rs:737` → :115 | start of log dir→READY: 18 / 23 ms (whole segment) | 11 / 18 ms | – | yes | P0 | Cheap; needed for diagnostics |
| A5 | `resolve_web_dist` | `src/serve/run.rs:70` | (in log dir→READY) | | 0.01 / 0.01 | yes | P0 | Static files must exist to serve the UI |
| A6 | `sweep_orphan_lsp_processes` (synchronous process-table scan) | `src/serve/run.rs:75` → `src/proc_lifetime.rs:48` | (in log dir→READY) | | 2.2 / 2.2 | yes | P2 | Only cleans up after crashed earlier runs; could run in the background. 2 ms is about 0.4–3 % of spawn→READY (§11) |
| A7 | `EngineManager::reconcile` | `src/serve/run.rs:84` | | | 0.4 / 0.4 | yes | P1 | Engine desired-state bookkeeping |
| A8 | `WorkspaceEngines::reconcile`: starts the warmup *threads* only | `src/serve/run.rs:85` → `src/engines/mod.rs:773/1105`, thread at :1171 | | | 0.1 / 0.1 | the spawn is serial; the work is background (Stage C) | P1 | |
| A9 | `path_index().attach` (background scan worker) | `src/serve/run.rs:86` → `src/engines/path_index/mod.rs:83` | | | 0.04 / 0.03 | the spawn is serial; the scan is background | P2 | Used only for path completion and search |
| A10 | `ServeState::new` (total) | `src/serve/run.rs:91` → `src/serve/state.rs:37/64` | | | 3.3 / 3.5 | yes | — | Sub-steps below |
| A10a | `WorkspaceWriteLease::acquire` | `src/serve/state.rs:84` | | | 0.02 / 0.02 | yes | P0 | One writer per workspace |
| A10b | `SessionData::open` (sessions.db) | `src/serve/state.rs:90` | | | 1.8 / 2.0 | yes | P0 | session/new and session/list need it |
| A10c | `list_session_ids_blocking` | `src/serve/state.rs:99` | | | 0.3 / 0.4 | yes | P2 | Only feeds A10d |
| A10d | **`maintain_snapshots`** | `src/serve/state.rs:97` → `src/session/snapshot.rs:1258` | | | 0.15 / 0.20 (7 sessions) | yes | P2 | Cleanup; could be background, but 0.2 ms is negligible at this size (§11) |
| A10e | `WorkspaceService::new` | `src/serve/state.rs:111` → `src/workspace/service.rs:97` | | | 0.01 / 0.02 | yes | P0 | Workspace file API |
| A10f | `TerminalHub::new` + `install_hub` (creates no PTY) | `src/serve/state.rs:112-113` | | | 0.001 / 0.001 | yes | P0 (empty registry) | PTYs are created on demand (C5) |
| A10g | `IdeBaseHandle::new`, `RuntimeHandle::new`, SessionManager / auto_turn installs | `src/serve/state.rs:114/120…` | | | not public, so not timed alone. Remainder of A10 minus A10a–f ≈ 0.7 / 0.8 | yes | P0 | Session's domain (B) |
| A11 | tokio runtime build | `src/serve/run.rs:110` | | | 3.6 / 0.1 (noisy 0.1–5.5) | yes | P0 | |
| A12 | **`restart_watcher`** (recursive OS watch registration, awaited) | `src/serve/router.rs:112` → `src/workspace/watcher.rs:270` | | | 4.0 / 5.0 | yes | P1 | Registering before serving means no edit made right after open is missed. 5 ms on 1,505 files; kept on path (§11) |
| A13 | background ticks (code_index_tick, session tick): spawned only | `src/serve/router.rs:122-234` | | | – | no | P2 | |
| A14 | `TcpListener::bind` + `LITECODE_READY` | `src/serve/router.rs:253-255` | READY = −2.3 to −8 ms (rel. T0) | | 0.04 / 0.04 | yes | P0 | |

**Stage A totals (logs, medians)**
- **spawn→log dir:** small 60–82 ms, big 379–566 ms.
- **log dir→READY:** 10–29 ms on both repos. That agrees with the harness steps A5–A14 (about 11–12 ms) plus logging setup.

### Stage B: READY / T0 → input box (t_int)

| # | Step | Source | Cold (small / big) | Hot reconnect (small / big) | Serial? | Class |
|---|---|---|---|---|---|---|
| B1 | WS upgrade and subscriptions | `src/serve/router.rs:71` → `src/client_protocol/ws.rs` | WS open 1.2 / 0.9 | 0.8 / 1.0 | yes | P0 |
| B2 | `handshake_frames` → `server_hello` | `src/client_protocol/ws.rs:339` → `controller/mod.rs:1190` → `project.rs:621` | hello 7.0 / 5.5 | 1.1 / 1.3 | yes | P0 |
| B3 | `session/new` reply = t_int (new profile) | `src/client_protocol/connection.rs` (SESSION_NEW) | 55.6 / 50.7 (about 45 ms server side) | (first connect) 58.5 / 59.3 | yes | P0 (Session's domain, B) |
| B4 | `session/list` reply = t_int (restore profile) | `src/client_protocol/connection.rs:891` | – | t_list 3.0 / 3.5 (reload 11.5 / 19.5) | yes | P0 (Session, B). The list was empty, see §6.2 |

### Stage C: after the input box (background and on demand)

| # | Step | Source | Cold median (small / big) | Hot median (small / big) | Serial? | Class |
|---|---|---|---|---|---|---|
| C1 | code_search worker spawn + handshake | `src/engines/code_search_engine.rs:374/399/438` | worker first seen −5 / −11 ms (around READY) | −3 / −13 | no | P1 |
| C2 | Worker index load: `CodeSearchIndex::load`, stamps, BM25, reconcile → **Warm** | `code_search_ipc/server.rs` (~:100), `store.rs:137` | Warm 1,297 / 1,229 (no index); with index 1,445 / 2,592 | 1,479 / 2,563 | no: t_int is the same with or without an index | P1 (the first search needs it) |
| C3 | LSP engine warmup = `LspHub::activate` availability probe (`npm config get prefix` + `<server> --version`) | `src/engines/lsp_engine.rs:48-73` → `src/lsp/hub.rs:202-208` → `src/lsp/deps.rs:490/317/237/129/423` | npm −5 / −5 ms; `--version` +399 / +565 ms | npm −3 / −5; `--version` +372 / +402 | no | P2 (§13) |
| C4 | Language server spawn + `initialize` (lazy, on the first LSP request) | `src/lsp/hub.rs:345/366/418` → `spawn_ls` :132 → `src/lsp/server.rs:144/544` | **not triggered** (the probe sends no LSP request); no timing | – | no | P2 |
| C5 | Terminal PTY (`terminal/create`, on demand) | `src/client_protocol/workspace.rs:74` | **not triggered**; no timing | – | no | P2 |
| C6 | Session sparse index rebuild | `engines/session_search/sparse` | log "sparse session index rebuilt" at about +1.1 s | – | no | P2 |

C4 and C5 were outside the probe's request set (plan R3, the front end's input-box definition), so first-request LSP and terminal latency are **not measured**. Measuring them would need a probe extension that sends `lsp/request` and `terminal/create` after t_int. I have not done this.

## 11. On the critical path but should be deferred (Andy's severity scale)

**One item:**

**A3 `warm_snapshot_repo` — severity (3), architecture design error**
- **Where:** `src/config/workspace.rs:513` → `src/session/snapshot.rs:1003`.
- **Why it is serial:** it runs synchronously inside configuration loading (`load_runtime_bundle`), before the logger is installed and before bind/READY. The call order is cli.rs:733 → manager.rs:146 → workspace.rs:513, then only afterwards cli.rs:737 `setup_logging` → `serve::run` → READY.
- **Why it can wait:**
  - Its consumer is the first turn's `snapshot_track` (P1), not READY or the input box.
  - `warm_snapshot_repo` and `snapshot_track` take the same `workspace_snapshot_lock` (snapshot.rs:112), so moving the warm to a background task after bind keeps the ordering correct: a first turn would simply wait on that lock.
- **Why "design" and not a plain bug:**
  - The placement is deliberate: the doc comment reads "Warm the shadow index on workspace open".
  - Its cost grows with the workdir file count (diff-files, untracked scan, add).
  - It sits on every open's start segment.
  - Its own "snapshot index warmed" log line is lost, because the logger does not exist yet, so the cost cannot be seen in production logs.
- **Size:**
  - About 300–480 ms (80–90 % of large-repo spawn→READY) on a first open for a HOME, and on projects without a usable git index (our big repo has a `git init` marker but no commits).
  - About 5 ms at steady state on 1,505 files.
  - Projects with a real git index (small, serde-json) are seeded and showed no git process lasting ≥ 10 ms.
- **Direction (language-generic, no new states):** run the warm once in the background after `TcpListener::bind`, under the existing lock. Move its log after logger setup.

**Serial steps checked and not listed, with the reason:**
- **A6 `sweep_orphan_lsp_processes`:** P2 and serial, but 2.2 ms. It could go to the background at no risk, but it is not material, so it is not graded.
- **A10d `maintain_snapshots`:** P2 and serial, 0.15–0.2 ms with 7 sessions. Its cost grows with the snapshot and session count. Not material at the measured size; worth a check with a large sessions.db.
- **A10c `list_session_ids`:** 0.3–0.4 ms; it only feeds A10d.
- **A12 `restart_watcher`:** P1, and registering before serving is a correctness requirement (no missed edits). 4–5 ms.
- **A10a/b/e/f, A1, A2, A4, A5, A14:** P0.
- **T0→t_int (Stage B):** nothing in the IDE's domain. t_int is mostly the `session/new` round trip (about 45 ms), which belongs to Session's W4.

## 12. Attribution of the extra ~400 ms before READY on the large repo

Medians per tier from the W4 logs and the /proc child timeline:
| repo | tier | spawn→log dir (process start + load_runtime_bundle incl. snapshot warm) | log dir→READY (setup_logging + serve::run to bind) | snapshot git span (first→last git child, rel spawn) | cs-worker first seen (rel T0) | npm config get prefix (rel T0) | rust-analyzer --version (rel T0) | cs Warm (rel T0) |
|---|---|---|---|---|---|---|---|---|
| small | cold_noindex | 69 | 18 | none ≥10 ms | -5 | -5 | 399 | 1297 |
| small | cold_noindex_tc | 82 | 21 | none ≥10 ms | -13 | -13 | 428 | 1389 |
| small | cold_index | 65 | 19 | none ≥10 ms | -11 | -11 | 373 | 1445 |
| small | cold_index_tc | 80 | 17 | none ≥10 ms | -3 | -3 | 387 | 1510 |
| small | hot_reconnect | 67 | 11 | none ≥10 ms | -3 | -3 | 372 | 1479 |
| small | hot_reload | 60 | 10 | none ≥10 ms | -2 | -2 | 423 | 1463 |
| big | cold_noindex | 508 | 23 | 93→494, 96→418, 93→539 | -11 | -5 | 565 | 1229 |
| big | cold_noindex_tc | 501 | 29 | 181→589, 102→473, 95→455 | -16 | -16 | 427 | 1281 |
| big | cold_index | 394 | 26 | 75→379, 73→376, 83→455 | -14 | -13 | 412 | 2592 |
| big | cold_index_tc | 566 | 21 | 91→545, 123→537, 86→476 | -15 | -14 | 497 | 2495 |
| big | hot_reconnect | 379 | 18 | 64→359, 74→377, 79→366 | -13 | -5 | 402 | 2563 |
| big | hot_reload | 442 | 19 | 75→422, 75→398, 72→640 | -18 | -18 | 426 | 2459 |

**Findings**
1. **The extra time all comes before the log dir is created.** log dir→READY is the same on both repos (10–29 ms). spawn→log dir is 60–82 ms on small and 379–566 ms on big. The log dir is created by cli.rs:737 `setup_logging`, so the extra cost sits inside cli.rs:733 `load_runtime_bundle`.
2. **Inside that window, git is running.** The snapshot `git` children (`--git-dir <HOME>/.litecode/snapshots/big_<id>`, from `git_cmd` at snapshot.rs:287) appear 64–181 ms after spawn. The last one appears 15–40 ms before the log dir. Before the first git, the elapsed time (64–95 ms) matches small's whole spawn→log dir. So the remaining `load_runtime_bundle` work is the same on both repos, and the git span (about 300–480 ms) is the difference.
3. **"load_runtime_bundle 373 ms" and "snapshot git" are the same cost, not two costs.** Harness r1 on big (the first open in `home-harness`) spent 373 ms in `load_runtime_bundle`. r2 and r3 spent 15.9 and 16.9 ms, against 12.3 ms on small. So the snapshot warm is about 357 ms on a first open and about 5 ms afterwards.
4. **It is serial, and it is in Stage A** (spawn→READY, before T0). The source order is in §11. It does not touch T0→t_int.
5. **Why every probe run paid it:**
   - The snapshot repo lives in the HOME, and the probe isolates the HOME per run, so every run was a first open.
   - Our big repo (a `git archive` copy plus `git init`, no commits) takes the unseeded path: `seed_from_project_git` finds no objects or index, so every file is bootstrapped as untracked.
   - Small has a real `.git`, so it is seeded with alternates and the index, and stays incremental.
6. **Resolution limits.** /proc is polled every 10 ms, so a git process shorter than 10 ms is not seen. The snapshot step's own info log is emitted before the logger exists and is lost. These limits do not change the attribution or the serial verdict.

**Data is sufficient; no extra measurement is required for the verdict.**

Optional confirmation, not run (plan §3 counterfactual: the same large repo with an already-warmed snapshot repo should show spawn→READY close to small's ~85 ms and no git child). It takes 4 light runs of about 40 s each and needs Session's W4 to have finished:
```
cd /workspace/litecode/.tmp/wp154
P=/workspace/litecode/.litecode/ops/artifacts/WP-155/probe/run.mjs; B=/workspace/litecode/target/release/litecode
node $P --bin $B --corpus $PWD/big --scenario hot_reconnect --profile new --hot-profile restore --runs 1 --out $PWD/raw/w4x/prep/rec.jsonl     # leaves raw/w4x/prep/home-hot_reconnect-1 with a warmed snapshot repo
cp -a raw/w4x/prep/home-hot_reconnect-1 raw/w4x/seedhome
node $P --bin $B --corpus $PWD/big --scenario hot_reconnect --profile new --hot-profile restore --runs 3 --seed-home $PWD/raw/w4x/seedhome --out $PWD/raw/w4x/warmhome/rec.jsonl
```
Guards are as in W4: MemAvailable ≥ 6 GiB and no cargo/rustc, checked by the probe before each run. Afterwards, delete `raw/w4x/*/home-*`.

## 13. Who started rust-analyzer at T0+495 ms, and whether cargo ran

**Correction.** The process I called a `cargo` child was `/workspace/litecode/.litecode/ops/.toolchain/cargo/bin/rust-analyzer --version`. That is the rustup proxy in CARGO_HOME/bin, which then executes the toolchain's `rust-analyzer --version`. My earlier process classifier cut the path short and read "cargo".
- Across all 36 W4 runs, the only children that appear after T0 are `rust-analyzer --version`: 26 times as the toolchain binary, 9 times as the proxy, plus 1 unnamed short-lived process.
- **There is no `cargo` process, no cargo metadata, and no language-server process** (no `rust-analyzer` without `--version`).
- So "a language server is spawned only on the first LSP request" holds: hub.rs:345/366/418 → `spawn_ls` was never reached.

**What triggered it: workspace engine warm-up at startup. Not a probe request, and not diagnostics.**
- The probe sends only `session/new`, `session/list`, the HTTP engines poll and `GET /`. It also writes `.litecode/engines.json` with `lsp.desired=true, servers=[rust-analyzer]`, which is what enables the LSP engine.
- Call chain:
  1. `serve::run` → `WorkspaceEngines::reconcile` (`src/serve/run.rs:85`)
  2. → `std::thread::spawn(engine.warmup())` (`src/engines/mod.rs:1171`)
  3. → `LspEngine::warmup` (`src/engines/lsp_engine.rs:48`; servers from engines.json at :61, commands at :67)
  4. → `LspHub::activate` (`src/lsp/hub.rs:202-208`), which checks each configured server
  5. → `deps::command_runnable_command_in(cmd, cwd = workspace root)` (`src/lsp/deps.rs:490`)
  6. → `resolve_server_binary` (:317) → `resolve_path_candidate` (:237) → `npm_bin_search_dirs` → **`npm config get prefix`** (:129; a node process, first seen around READY, −18 to −2 ms)
  7. → **`run_version_probe`** (:423), which runs `<server> [args] --version` from the project root "so proxies (rustup, mise, volta) pick the project's toolchain". That is the process seen at +372 to +565 ms.
- **The code is language-generic:** it runs for every configured server and is not special-cased for Rust. `adapters/rust_analyzer.rs` only matters for managed installs.
- **Classification: Stage C, background, P2, not on the critical path.** It runs on its own thread, starts after t_int, and t_int does not change with or without it.

**Side effects and a contradiction**
- **Doc comment is wrong:** lsp_engine.rs:71 says "activate is metadata-only (no language-server I/O)", but `activate` executes one npm process plus one proxy chain on every open.
- **Proxies can do heavy work at open:** the probe runs from the project directory, so a version manager there (rustup, mise, volta, asdf) may resolve, or even download, a toolchain pinned by the project (`rust-toolchain.toml`, `.tool-versions`, …) just because the workspace was opened. The same holds for any language whose server goes through such a proxy.
- **npm runs even for servers that npm does not distribute,** just to locate them.

**Direction (language-generic, no new states; not graded in §11 because it is off the critical path)**
- **Like VS Code** (an extension starts its client and server on `onLanguage` activation) **and Zed** (a server starts when a buffer of that language opens in the worktree; the binary is found through the worktree's shell environment and cached):
  - Move the availability check to the first need: the first LSP request, or the first file of that language.
  - On open, only record the configured commands.
- If engine status needs a "runnable" flag before first use:
  - Resolve by PATH lookup or the managed directory without executing anything.
  - Cache the `--version` result in the HOME, keyed by resolved path + mtime.
  - Run it on a low-priority background task.
- Query the npm prefix only for servers whose adapter metadata says they are npm-installed. Cache it once per process.
- Fix the lsp_engine.rs:71 comment, or make it true.

**W3 note:** the same probe appeared during W3 as `/bin/sh /usr/local/bin/rust-analyzer --version` (the /usr/local/bin shim leading into the same rustup proxy).

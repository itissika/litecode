# WP-154 — Startup work checklist, back end (backend.md, draft by ide)

**Scope:** the `litecode serve` process, from spawn until it goes idle. Session's `hello / session/list / session/new / session/subscribe` and its session-index loading are not covered here (see session.md).

**Method:** source reading only. No compiling, nothing run. Every code location was re-checked against the restored repo on 2026-10-15 at about 01:40 HKT, HEAD 3ddc6433.

**Measurements:** the measurement work is cancelled; WP-154 has changed direction. The measured times below are quoted from the WP-155 artifact `.litecode/ops/artifacts/WP-155-ide.md`, with the section number given. "Not measured" means WP-155 has no number for it.
- Unless stated otherwise, times are medians written small repo / large repo (93 / 1,505 files).
- The harness figures come from a HOME that was reused across runs.

## How the stages are defined
| Stage | Current position | Recommended stage |
|---|---|---|
| **P0** | Runs serially before READY | Must be done before the workspace opens |
| **P1** | Starts automatically in the background after startup | Do right after opening |
| **P2** | Triggered on demand | Can be deferred or run on demand |

## (a) Items whose recommended stage differs from where they run now

1. **`warm_snapshot_repo` (#3): P0 → P1.**
   - **Where:** `src/config/workspace.rs:513` → `src/session/snapshot.rs:1003`.
   - **Why it is P0 today:** it runs synchronously inside `load_runtime_bundle`, before logging is set up and before bind.
   - **Who needs it:** only the first turn's `snapshot_track` (snapshot.rs:1023/1029), which takes the same `workspace_snapshot_lock` (snapshot.rs:112). Running the warm in the background after bind keeps the ordering correct.
   - **Cost:** it starts several git subprocesses (#3a, #3b), and their cost grows with the number of workdir files.
   - **Measured:** about 300–480 ms on a first open, about 5 ms on later opens (WP-155 §11, §12).
2. **`EngineManager::reconcile` (#9): P0 → P2.**
   - **Where:** `src/serve/run.rs:84` → `src/optional/mod.rs:149`.
   - **Why it is P0 today:** no tokio runtime exists yet at this point, so the `else` branch at optional/mod.rs:230 calls `engine.warmup()` synchronously on the main thread. That builds the webfetch and websearch `reqwest` blocking clients (TLS configuration) (`src/optional/webfetch_engine.rs:28`, `websearch_engine.rs:42`).
   - **Who needs it:** only a webfetch or websearch tool call, so the clients can be built on first use.
   - **Measured:** 0.4 ms (§5).
3. **`sweep_orphan_lsp_processes` (#8): P0 → P1, in the background.**
   - **Where:** `src/serve/run.rs:75` → `src/proc_lifetime.rs:48`.
   - **What it does:** up to 3 passes over the whole process table (Linux `sweep_once` at :215; Windows ToolHelp at :128).
   - **Why it can move:** it only cleans up servers left behind by a crashed earlier run. Nothing before READY depends on it. If it is moved, it only has to finish before the first language-server spawn (#27).
   - **Measured:** 2.2 ms (§5).
4. **`maintain_snapshots` (#16, plus the `list_session_ids_blocking` call #15 that feeds it): P0 → P1, in the background.**
   - **Where:** `src/serve/state.rs:97-99` → `snapshot.rs:1258`.
   - **What it does:** cleanup only. It opens the shadow repo and walks every session's refs and patch directories.
   - **Why it can move:** nothing before READY depends on it.
   - **Caveat:** it does **not** take `workspace_snapshot_lock` today. If it moves to the background, it has to take that lock so it does not race the first turn's snapshot (#3).
   - **Measured:** 0.15–0.2 ms with 7 sessions (§5).
5. **`remove_legacy_text_index`, inside `path_index().attach` (#11): P0 → P2.**
   - **Where:** `src/serve/run.rs:86` → `src/engines/path_index/mod.rs:98/400`.
   - **What it does:** a one-time `remove_dir_all` of the old `.litecode/text-index`. That is a migration cleanup, so it can run in the background.
   - **Measured:** the whole `attach` call takes 0.03–0.04 ms (§5). The removal itself only does work when the legacy directory exists.
6. **The LSP availability probe (#24–#26, `npm config get prefix` + `<server> --version`): P1 → P2.**
   - **Where:** `src/engines/lsp_engine.rs:48/74` → `src/lsp/hub.rs:202` → `src/lsp/deps.rs:490/317/237/165/129/423`.
   - **What happens today:** every open starts one node process plus one `--version` call per configured server, run from the project directory. A version manager there (rustup, mise, volta) may resolve, or even download, a toolchain the project pins.
   - **Recommendation (language-generic):** check availability on first need (the first language-server spawn, #27). This is the approach VS Code (`onLanguage` activation) and Zed (start when a buffer of that language opens) take.
   - **Measured:** npm appears at about −18 to −2 ms; `--version` at +372 to +565 ms after the WebSocket upgrade request (§10 C3, §13).
   - **Also:** the comment at lsp_engine.rs:73 says activate is "metadata-only", but it starts processes.

Every other row in the table already sits in its recommended stage.

## (b) Items that must stay P0 and that the code shows to be slow

**None.** Every item that must stay P0 was checked:
- **Fixed cost, independent of repo size:**
  - Config and global db: #1 and #2. These read and write a few SQLite and JSON files.
  - Workspace layout and identity: #4.
  - `setup_logging`: #6.
  - `WorkspaceWriteLease`: #13.
  - `WorkspaceService::new`: #17.
  - `TerminalHub` and `install_hub`: #18.
  - `TcpListener::bind` and READY: #23.
- **Session's domain:** `SessionData::open` (#14) belongs to Session (see session.md). The measured cost is 1.8–2.0 ms (§5).
- **The one P0 item whose cost grows with workspace size is `restart_watcher` (#21):**
  - **Where:** `src/serve/router.rs:112` → `src/workspace/watcher.rs:270/274` → `WorkspaceWatcher::start` :30, which calls `watcher.watch(root, RecursiveMode::Recursive)` at :43.
  - **Why it grows:** on Linux, inotify adds one watch per directory by walking the whole tree synchronously, so the cost grows with the number of directories.
  - **Why it stays P0:** registering before serving means no edit made right after opening is missed (the change bus feeds #19 and #33 and the explorer).
  - **Measured:** 4.0 / 5.0 ms on 93 / 1,505 files (§5). That is not slow at the measured scale.
  - **Status:** it is not counted as slow. It is recorded here as the only scale-dependent P0 item, and should be re-checked on a repository with tens of thousands of directories (not measured).
- **The slow items found by reading the code are all recommended for P1/P2** and are listed in (a): #3 snapshot warm, and the git, npm and `--version` subprocesses.

## Full checklist (one item per row, following the order of the call chain)

| # | Work item | Code location | What it depends on | What depends on it | Current stage | Recommended stage | One-line reason | Measured time |
|---|---|---|---|---|---|---|---|---|
| 1 | CLI parse; `code-search-worker` subcommand branch | `src/cli/cli.rs:701-706` | process start | everything below | P0 | P0 | Picks the run mode | part of spawn→log dir: 60–82 / 379–566 ms, which includes #2–#3 (§12) |
| 2 | `load_runtime_bundle`: provider catalog, `migrate_once`, `repair_agent_models_on_boot`, `load_global_from`, `validate_structural`, `resolve`, model warnings | `src/cli/cli.rs:733/93` → `src/config/manager.rs:131-150` | global litecode.db | all config consumers | P0 | P0 | Config and providers are needed by everything | 10–12 ms on later opens (§5 r2/r3, including #4); 373 ms on the first open of the large repo is almost all #3 (§12) |
| 3 | **`warm_snapshot_repo`** (takes `workspace_snapshot_lock`) | `src/config/workspace.rs:513` → `src/session/snapshot.rs:1003` | #4 (snapshots_dir), `git` on PATH | the first turn's `snapshot_track` (snapshot.rs:1023) | P0 | **P1** | Only the first turn needs it; same lock, so moving it to the background is safe | ≈300–480 ms first open; ≈5 ms after (§11, §12) |
| 3a | ├ `open_or_init_repo` (libgit2 open/init; on init: `configure_snapshot_repo` runs 4 `git config` subprocesses, `seed_from_project_git` runs `git rev-parse` and writes alternates + copies the index; `sanitize_shadow_alternates`) | `snapshot.rs:140-173`, :175-193, :366 (`git_config_set`), :382 | #3 | #3b | P0 | P1 | The subprocesses only happen on the first open per HOME | not measured separately; included in #3 |
| 3b | ├ `cli_sync_and_write_tree`: git subprocesses `diff-files`, `ls-files --others --exclude-standard`, `check-ignore`, `rm --cached`, `add`, `write-tree` | `snapshot.rs:563/584-700` (`git_cmd` :287) | #3a | #3 | P0 | P1 | Cost grows with workdir file count and untracked files | git spans 64–181 → 359–640 ms after spawn on the large repo (§12) |
| 4 | `load_workspace_state`: resolve root, `init_workspace`, `ensure_workspace_identity`, `set_runtime_paths`, create snapshots dir, engines readiness, workspace defs (mcp/custom tools), `read_contract` | `src/config/workspace.rs:491-511` (via `manager.rs:146/163`) | #2 | #3, #6, #13–#20, engines | P0 | P0 | Workspace paths, identity and the sessions.db path are needed first | not measured separately (included in #2) |
| 5 | `setup_logging` (log dir, file appender, level from db) | `src/cli/cli.rs:737` → :115 | #4 (logs_dir) | diagnostics | P0 | P0 | Cheap, and needed for diagnostics; note that #2–#4 log nothing because they run before it | included in log dir→READY: 10–29 ms (§10, §12) |
| 6 | Bind address and auth check (`validate_serve_bind`, `resolve_auth_token`) | `src/serve/run.rs:56-67` | CLI options | #23 | P0 | P0 | Security check before serving | not measured (computation only) |
| 7 | `resolve_web_dist` | `src/serve/run.rs:70` | env / install directory | static UI | P0 | P0 | The UI files must be present | 0.01 ms (§5) |
| 8 | `sweep_orphan_lsp_processes` (up to 3 passes over the process table) | `src/serve/run.rs:75` → `src/proc_lifetime.rs:48` (Linux :215, Windows :128) | lsp_dir | must finish before #27 | P0 | **P1** | Cleans up after crashes; can run in the background | 2.2 / 2.2 ms (§5) |
| 9 | `EngineManager::reconcile`: synchronous webfetch/websearch warmup (builds reqwest blocking clients), because no runtime exists yet | `src/serve/run.rs:84` → `src/optional/mod.rs:149/165/230`; `webfetch_engine.rs:28`, `websearch_engine.rs:42` | #2 | webfetch/websearch tools | P0 | **P2** | Only a web tool call needs it; build on first use | 0.4 / 0.4 ms (§5) |
| 10 | `WorkspaceEngines::reconcile`: reads engines.json; `start("code_search")` / `start("lsp")` → `std::thread::spawn(warmup)` (no runtime yet) | `src/serve/run.rs:85` → `src/engines/mod.rs:773/1105/1172` | #4, engines.json | #24–#33 | P0 (only starts the threads) | P0 | Starting the threads costs ~0; the work moves to the background | 0.1 / 0.1 ms (§5) |
| 11 | `path_index().attach`: `remove_legacy_text_index` (synchronous) + starts the `path-index` thread | `src/serve/run.rs:86` → `src/engines/path_index/mod.rs:83/98/400` | #4 | #34 | P0 | P0 to start the thread; **P2** for the legacy removal | The legacy cleanup is a one-time migration | 0.03–0.04 ms (§5) |
| 12 | `ServeState::new` → `with_project` (total) | `src/serve/run.rs:91` → `src/serve/state.rs:37/64` | #2, #4, #9, #10 | router and handlers | P0 | P0 | Sub-steps below | 3.3 / 3.5 ms (§5) |
| 13 | ├ `WorkspaceWriteLease::acquire` | `src/serve/state.rs:84` | .litecode dir | #14 | P0 | P0 | One writer per workspace | 0.02 ms (§5) |
| 14 | ├ `SessionData::open` (sessions.db) | `src/serve/state.rs:90` | #13 | Session (see session.md) | P0 | P0 | Session needs it; Session's domain | 1.8 / 2.0 ms (§5) |
| 15 | ├ `list_session_ids_blocking` (only feeds #16) | `src/serve/state.rs:99` | #14 | #16 | P0 | P1 (moves with #16) | Has no other consumer | 0.3 / 0.4 ms (§5) |
| 16 | ├ **`maintain_snapshots`** (walks session refs and patch directories; deletes orphaned and stale ones) | `src/serve/state.rs:97` → `src/session/snapshot.rs:1258` | #15, the shadow repo | none (cleanup) | P0 | **P1** | Cleanup only; in the background it must take `workspace_snapshot_lock` | 0.15 / 0.20 ms with 7 sessions (§5) |
| 17 | ├ `WorkspaceService::new` | `src/serve/state.rs:111` → `src/workspace/service.rs:97` | project root | #21, workspace API | P0 | P0 | Workspace file API and change bus | 0.01 / 0.02 ms (§5) |
| 18 | ├ `TerminalHub::new` + `install_hub` (empty registry, no PTY) | `src/serve/state.rs:112-113` → `src/terminal/mod.rs:157/158` | – | #35 | P0 | P0 | Empty registry, so ~0 cost | 0.001 ms (§5) |
| 19 | ├ `IdeBaseHandle::new`, `RuntimeHandle::new` (creates an empty MCP pool and does not connect), SessionManager, `install_*_auto_turn`, `set_session_reader` | `src/serve/state.rs:114-176`; `src/runtime/mod.rs:83` | #13–#18 | Session and the runtime (see session.md) | P0 | P0 | Wiring only; MCP connects on demand (runtime domain) | the rest of #12 ≈ 0.7 / 0.8 ms (§10 A10g) |
| 20 | tokio multi-thread runtime build | `src/serve/run.rs:110-112` | – | #21–#23, all async work | P0 | P0 | Needed for async serving | 0.1–5.5 ms, noisy (§5) |
| 21 | **`restart_watcher`**: `spawn_watcher` → `WorkspaceWatcher::start` (recursive watch registration, synchronous) + debounce `tokio::spawn` | `src/serve/router.rs:112` → `src/workspace/watcher.rs:270/265/30/43/48` | #17, #20 | #33, #34, #19 change bus, explorer | P0 | P0 | Registering before serving means no edit is missed; cost grows with directory count, see (b) | 4.0 / 5.0 ms (§5) |
| 22 | Starts the knowledge index thread `knowledge-index` (preview, full build, then a sweep every `SWEEP_EVERY` = 2 s, knowledge/index.rs:40) | `src/serve/router.rs:115` → `src/knowledge/index.rs:1289/1302/1207` | #17 | knowledge context | P0 (starting it) + P1 (the build) | P1 | Background build that never blocks the first turn (comment at router.rs:113) | not measured |
| 23 | `TcpListener::bind` → `LITECODE_READY` printed to stdout → `axum::serve` | `src/serve/router.rs:252-259` | #20, #21 | the front end connecting (see session.md for the WS handshake onward) | P0 | P0 | Readiness signal | 0.04 ms (§5); READY is 2–13 ms before T0 (§3) |
| 24 | **LSP warmup thread**: `LspEngine::warmup` (servers from engines.json, `commands_for_server_ids`) → `hub.activate` | thread from #10 → `src/engines/lsp_engine.rs:48/74` → `src/lsp/hub.rs:202` | #10, engines.json | the engine status shown as Warm/Failed | P1 | **P2** | Availability can be checked on first need | Warm at about +0.4–0.6 s (§10 C3) |
| 25 | ├ subprocess `npm config get prefix` (searches npm bin directories for the server binary) | `src/lsp/deps.rs:490` → :317 → :237 → :165 → :129 | #24 | #26 | P1 | **P2** | Only needed for servers distributed through npm, and the result could be cached | first seen at about −18 to −2 ms from T0 (§10, §13) |
| 26 | ├ subprocess `<server> --version` (from the project directory, through proxies such as rustup/mise/volta) | `src/lsp/deps.rs:423` (`run_version_probe`) | #25 | #24 | P1 | **P2** | Version managers can do toolchain work just because the workspace was opened | first seen at +372 to +565 ms (§10 C3, §13) |
| 27 | **Language server spawn + `initialize`** (lazy) | `src/lsp/hub.rs:220` (`resolve_server_key`) → :345/366/418 → `spawn_ls` :132 → `src/lsp/server.rs:144` (`Command::new`), :461 (`initialize`) | #24 (hub active), #8 | `lsp/request` (`src/client_protocol/workspace.rs:83`), agent LSP tools (`src/tools/lsp.rs`), post-edit diagnostics (`hub.rs:722/737`), `sync_document`/`apply_document` (:472/495) | P2 | P2 | Already on demand | not measured (no LSP request was sent; §10 C4) |
| 28 | LSP diagnostics subscription per WS connection | `src/client_protocol/ws.rs:282-283` | #23 | front-end diagnostics | P2 (per connection) | P2 | Subscription only, starts no process | not measured |
| 29 | **code_search warmup thread**: `kill_client` → `spawn_ready_worker` (child process `litecode code-search-worker`) → `handshake_worker` (initialize + warmup IPC) → `set_session_db` | thread from #10 → `src/engines/code_search_engine.rs:374/399/457/438`; `src/engines/code_search_ipc/client.rs:27-34/64` | #10, engines.json, the `litecode` executable | #30–#33, the first search | P1 | P1 | The first search needs it, and it is off the critical path | worker first seen at −18 to −2 ms; Warm 1.2–1.5 s without an index, 2.4–2.6 s with the large index (§2, §10 C1–C2) |
| 30 | ├ worker: `reload_worker_excludes`, `init_workspace_index`, `reclaim_stale_index_job` | `src/engines/code_search_ipc/server.rs:87-99` (:25); `src/engines/code_search/meta.rs:87`; `index_status.rs:188` | #29 | #31 | P1 | P1 | Index directory setup | not measured separately |
| 31 | ├ worker: `open_production_embedder` (loads the ORT model) | `server.rs:100` → `src/engines/code_search/embed.rs:319` | model files | #32 | P1 | P1 | Embedding needs the model | included in Warm without an index ≈ 1.2–1.5 s (§10 C2) |
| 32 | ├ worker: `warmup_index` (`CodeSearchIndex::load`: usearch restore, log replay, repair; or `start_fresh_index`) + `CodeSearchRuntime::new` (stamps, BM25) | `server.rs:101-106` → `src/engines/code_search/mod.rs:813`, `store.rs:137` | #31 | search | P1 | P1 | Loading grows with index size | +≈1.2 s for the 6,257-chunk index (§2, §10 C2) |
| 33 | ├ worker: `queue_reconcile_dirty` (full stat scan of the workdir against the stamps; writes queue.json) | `server.rs:110` → `src/engines/code_search/reconcile.rs:87` | #32 | #36 (incremental embedding) | P1 | P1 | Detects files changed while closed; cost grows with file count, in the background | not measured separately |
| 33s | ├ worker: session semantic index (`ensure_session_index` / `consume_session_index`) | `server.rs:111-118` | #29 | Session search | P1 | see session.md | Session's domain | – |
| 34 | `path-index` thread: `fill` (full path scan), then waits for updates or a rescan | `src/engines/path_index/mod.rs:229-260` | #11, #21 | path completion and search | P1 | P1 | Background; grows with file count | not measured |
| 35 | **Terminal PTY** (`terminal/create` → `TerminalHub::create` → `openpty` + shell process + `pty-reap-*` thread) | `src/client_protocol/workspace.rs:136` → `src/terminal/mod.rs:297`, `src/terminal/pty.rs:300`, reaper thread `terminal/mod.rs:254` | #18, #23 | terminal UI, `bash` tool / auto-turn | P2 | P2 | Already on demand | not measured (§10 C5) |
| 36 | `tokio::spawn` code-index tick (`CODE_INDEX_TICK` = 30 s; the first tick runs immediately): stat scan through worker IPC, or `queue_reconcile_from_stamps` in the serve process when no worker is running; when Warm and there is work, `spawn_scoped_refresh` (embedding refresh) | `src/serve/router.rs:35/213-228` → `src/engines/mod.rs:934/1096`; `reconcile.rs:124`; `index_status.rs:101` | #20, #29 | index freshness, first-build embedding | P1 | P1 | Background; the first tick usually lands while the worker is still Warming, so the first embedding starts at the second tick, about 30 s in (W3: chunks.total first appeared at +31 s, §7) | first embedding ≈ +30 s; first build 1,103 s / 6,257 chunks (§7) |
| 37 | `tokio::spawn` change-bus subscriber: knowledge `notify_paths`, reload of `mcp.json` / custom tools, `code_search().notify_fs_changes`, `path_index` notify, `request_index_sync`; on lag: reconcile and rescan | `src/serve/router.rs:116-165` | #21 | #22, #29, #34 | P1 (event-driven) | P1 | Keeps the indexes current | not measured |
| 38 | `tokio::spawn` rescan subscriber (watcher error / overflow → `request_reconcile` + `request_rescan`) | `src/serve/router.rs:230-249` | #21 | #29, #34 | P1 (event-driven) | P1 | Recovers after lost events | not measured |
| 39 | `tokio::spawn` session GC + session index tick (`SESSION_INDEX_TICK` 30 s, sparse maintenance, dense consume) | `src/serve/router.rs:166-210` | #19 | Session | P1 | see session.md | Session's domain | – |
| 40 | Shutdown watch (stdin EOF / parent-pid poll) inside the `axum::serve` graceful shutdown | `src/serve/router.rs:260-266` → `src/serve/shutdown.rs:67/81` | CLI options | shutdown | P1 | P1 | Process lifetime | not measured |

**Count:** 43 rows (#1–#40 plus the sub-rows 3a, 3b and 33s).

### Background threads, tasks and child processes, until the process is idle
- **Threads:**
  - #10 → the code_search warmup thread (#29) and the LSP warmup thread (#24)
  - `path-index` (#34)
  - `knowledge-index` (#22)
  - `pty-reap-<id>` (on demand, #35)
  - tokio runtime workers and blocking pool (#20; ticks run in `spawn_blocking`)
- **tokio tasks:**
  - watcher debounce (#21)
  - change bus (#37)
  - rescan (#38)
  - session GC + session tick (#39, see session.md)
  - code-index tick (#36)
  - shutdown watch (#40)
  - one diagnostics subscription per WS connection (#28)
- **Child processes:**
  - the git subprocesses of #3 (on the first open: 4× `git config` + `git rev-parse`; every open: diff-files, ls-files, check-ignore, (rm), add, write-tree)
  - `npm config get prefix` (#25)
  - `<server> --version` (#26)
  - `litecode code-search-worker` (#29, long-lived)
  - on demand: language servers (#27) and the PTY shell (#35)
- **When the process is idle:** after #29 is Warm, #34's scan and #22's build have finished, and #36 has no pending work, which includes any first-build embedding. What is left are the 30 s ticks and the event-driven tasks.

## Where this disagrees with WP-155 (`WP-155-ide.md`), and the corrections
1. **WP-155 §10 A7 (`EngineManager::reconcile`) was wrong.** It called the step "engine desired-state bookkeeping" and placed it at P1. In fact, before the tokio runtime exists, it synchronously runs the webfetch/websearch warmup on the main thread, building HTTP clients (optional/mod.rs:230). The current stage is P0, and the recommendation here is P2.
2. **WP-155 §10 A10d recommended keeping `maintain_snapshots` on the critical path** ("negligible"). This checklist recommends P1, and adds a requirement WP-155 missed: moved to the background, it has to take `workspace_snapshot_lock`.
3. **WP-155 §10 A6 already classed `sweep_orphan_lsp_processes` as P2**, saying it could run in the background. This checklist puts it at **P1**, because it must finish before the first language-server spawn (#27).
4. **Items WP-155 did not list:**
   - the knowledge index thread (#22)
   - the change-bus and rescan tasks (#37, #38)
   - the code-index tick (#36)
   - the worker's full stat scan `queue_reconcile_dirty` (#33)
   - `remove_legacy_text_index` (#11)
   - the 4 `git config` subprocesses plus `git rev-parse` that run on snapshot init (#3a)
   - the shutdown watch (#40)
5. **New explanation of the W3 timing:** the first embedding is started by #36's second tick, not by the warmup itself. That explains why `chunks.total` first appeared at +31 s in W3 (WP-155 §7 only recorded the time).
6. **No other conflicts.** The conclusions and numbers of WP-155 §11 (`warm_snapshot_repo`, severity (3)), §12 (attribution of the extra ~400 ms) and §13 (`--version` probe, no cargo) still hold. The Session rows from WP-155 §10 (A10g, B3, B4, C6) are left to session.md.

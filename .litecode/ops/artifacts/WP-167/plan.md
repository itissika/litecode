# WP-167 启动排位整改 · IDE 域 · 实施方案（ide，只读起草）

依据：board WP-165（GM 已批拆分）/ WP-167 goal；`.litecode/ops/artifacts/WP-154/backend.md` §(a) 第 2/3/5/6 条及表 #8/#9/#11/#24–#27。
基线：HEAD 3ddc643，工作区 src 与 HEAD 一致（rollback-loss-ide.md）。行号均为当前工作区，只读核实。
约束：功能行为不变；语言通用、不加单语言特例；不加新状态（只用现有 EngineState::{Warming,Warm,Failed,Stopped} 与「未出现在 map」）；工具可见性只用 bind（不改）；批准前不改代码、不编译。

## 0. 归属核对
| 位置 | 归属（board domains.place） | 本包处理 |
|---|---|---|
| src/lsp/**, src/engines/**（含 path_index、lsp_engine、status_view） | ide（src/{workspace,terminal,lsp,engines}） | 我落地 |
| src/serve/run.rs（serve::run 调用点） | session（src/{session,client_protocol,serve}） | 跨域 diff，交 owner |
| src/optional/**（EngineManager）、src/tools/**（webfetch/websearch/lsp 工具）、src/tool/registry.rs | tools（src/{tool,tools,…,optional}） | 跨域 diff，交 owner |
| src/proc_lifetime.rs | 不在任何 domain.place 中（crate 根文件，最近由 release 提交 70c91f8 触及） | **不改**；只换调用方式 |
| src/cli/cli.rs:803 `engine_manager.reconcile` | runtime（兼 cli/optional） | 不动（CLI 不在本次启动路径） |
注：path_index 在 src/engines/path_index/，属 ide，第 3 项全部在我域。

---

## 1. sweep_orphan_lsp_processes：P0 → P1 后台，第一个 language server 启动前必完成

### 现状
- src/serve/run.rs:73-75 同步调用 `crate::proc_lifetime::sweep_orphan_lsp_processes()`（≤3 轮扫进程表，proc_lifetime.rs:48）。
- 所有 language server 进程只有一个出生点：`LspHub::spawn_ls`（src/lsp/hub.rs:132-142 → `LspServer::spawn`，server.rs:137；`rg "LspServer::spawn"` 仅此一处）。它的 3 个调用点：首启 hub.rs:366、自动重启 :345、手动 `restart_server` :418。

### 改动
1. 新文件 **src/lsp/sweep_gate.rs**（我域）：
   ```rust
   pub struct SweepGate { state: Mutex<GateState>, cv: Condvar, tx: tokio::sync::watch::Sender<bool> }
   enum GateState { Unarmed, Running, Open }   // 内部私有枚举，非引擎状态
   pub fn global() -> &'static Arc<SweepGate>  // static OnceLock
   impl SweepGate {
       /// serve 启动时调用：置 Running，起线程跑 sweep；线程起不来就当场同步跑（= 今日行为）。
       pub fn start_background(self: &Arc<Self>, sweep: fn())
       /// spawn_ls 前 await；Unarmed（CLI/测试未走 serve）与 Open 立即返回。
       pub async fn wait(&self)
   }
   struct OpenOnDrop(Arc<SweepGate>);           // Drop 里无条件 open()
   ```
   线程体：`let _g = OpenOnDrop(gate.clone()); let _ = std::panic::catch_unwind(AssertUnwindSafe(sweep));` —— 正常返回、提前 return、panic 三条路径都经 Drop 打开门闩；`catch_unwind` 让 panic 不传播、只 `tracing::warn!`。`open()` 用 `Mutex` 置 Open + `watch::send_replace(true)`；`Mutex` 中毒用 `into_inner()`（与仓库现有写法一致），不会因中毒而永不打开。`thread::Builder::spawn` 失败分支：同步执行 sweep（同样包在 OpenOnDrop 里）。**不设超时**：超时放行会破坏「先 sweep 后 spawn」保证；sweep 本身 2.2 ms（WP-155 §5），极端情况下只延后首次 spawn，不死锁。
2. **src/lsp/hub.rs:132 `spawn_ls`** 开头加 `self.sweep_gate.wait().await;`（`LspHub` 持 `Arc<SweepGate>`，`new()` 默认取 `sweep_gate::global()`；测试用 `with_sweep_gate_for_test`）。因三条 spawn 路径都经此处，「第一个 LS 启动前 sweep 完成」由单一阻塞点保证。
3. **跨域（Session）src/serve/run.rs:73-75**：一行替换
   `crate::proc_lifetime::sweep_orphan_lsp_processes();` → `crate::lsp::sweep_gate::global().start_background(crate::proc_lifetime::sweep_orphan_lsp_processes);`
   位置不变（仍在 `workspace_engines.reconcile` 之前 arm，故任何 LSP warmup/spawn 都看得到 Running）。

### 行为不变的理由
- sweep 的判定与杀进程逻辑（proc_lifetime.rs）一字不改；只是从主线程挪到 `lsp-orphan-sweep` 线程。
- 今日顺序「sweep 完 → 之后才可能 spawn」由门闩保持；被扫的只是父进程已死的旧 LS，本进程新起的 `--version` 探测子进程父进程存活，不在扫描范围，故探测不需等门闩（与今日并发关系一致）。
- CLI / 单测从不 arm → `wait()` 直通，等同今日（今日 CLI 也不 sweep）。

### 测试
| 测试名 | 断言 |
|---|---|
| `lsp::sweep_gate::tests::wait_passes_when_never_armed` | 新建 gate 未 arm，`wait()` 在 50 ms 内完成 |
| `…::wait_blocks_until_sweep_returns` | sweep 闭包阻塞在 channel 上；`wait()` 200 ms 内未完成；放行后 1 s 内完成 |
| `…::gate_opens_when_sweep_panics` | sweep `panic!()`；`wait()` 1 s 内完成；测试进程不 panic |
| `…::second_start_is_noop_after_open` | 已 Open 时再 `start_background` 不再调用 sweep（计数器 = 1） |
| `lsp::hub::tests::spawn_ls_waits_for_sweep_gate` | 自有 gate 处 Running；配置一个 mock LS（沿用 conn.rs 的 mock 脚本手法，启动即写 marker 文件），发起 spawn；放行前 marker 不存在；放行后存在 |
| `tests/startup_order_death_list.rs::serve_run_does_not_sweep_inline` | 源码扫描 `serve::run` 函数体不含 `sweep_orphan_lsp_processes()` 直接调用；`spawn_ls` 函数体含 `sweep_gate.wait()`（标识符边界匹配，沿用 WP-051 C1/C2 做法） |

---

## 2. EngineManager::reconcile（webfetch/websearch HTTP client）：P0 → P2（首次使用时建）——**全部跨域（Tools + Session）**

### 现状
- src/serve/run.rs:84 `engine_manager.reconcile(&resolved)`，此时尚无 tokio runtime，走 src/optional/mod.rs:230 `else` 分支在主线程同步 `warmup()`，建 reqwest blocking client（webfetch_engine.rs:28、websearch_engine.rs:42）。
- 消费方：工具在 client 为 `None` 时直接报错 `"webfetch engine not warmed"`（src/tools/webfetch.rs:22-29）/ `"websearch engine not warmed"`（src/tools/websearch.rs:25-31）。`EngineManager::engine_status` 无调用方（rg 只有定义 optional/mod.rs:142），UI 不展示其状态。
- 可见性：`should_include_in_llm_list`（src/tool/availability.rs）不读 EngineManager（参数 `_engines`），webfetch 为 core，无需 bind 以外任何门控 —— 不变。

### 改动（最小化，交 Tools 落地；Session 只删/换一行）
1. **Tools · src/optional/mod.rs:149 附近**新增
   - `pub fn defer_reconcile(&self, resolved: &ResolvedConfig)`：保存 `desired: HashSet<&'static str>`（按 `should_run_engine`）与 `websearch.configure(resolved.websearch())`（只写 endpoint 字符串，零成本，保证 `"websearch is not configured"` 文案路径不变）；不建 client、不写 states（map 中缺省 = 今日「从未启动」，非新状态）。
   - `pub fn ensure_warm(&self, id: &str)`：若 client 已在 → 返回；若 `id ∈ desired` → 走既有 `start_warmup` 的同步 `else` 分支逻辑（抽成私有 `warm_now`）。用一把 `Mutex<()>` 防并发双建。
   - 既有 `reconcile`（settings 变更时 runtime/mod.rs:185、settings_writer.rs:293 调用）不改 → 设置变更后仍立即建，行为同今日；`reconcile` 同时刷新 `desired` 集合，保证之后 `ensure_warm` 判定与最新配置一致。
2. **Tools · src/tool/registry.rs:130/134**：把 `engines.webfetch_ensure()`（`Arc<dyn Fn() + Send + Sync>`，内部调 `ensure_warm("webfetch")`）随 client handle 一起传入工具构造。
3. **Tools · src/tools/webfetch.rs:22 / websearch.rs:25**：读 client 为 `None` 时先调一次 ensure，再读；仍为 `None` 才返回原文 `"... engine not warmed"`。（各 +2 行）
4. **Session · src/serve/run.rs:84**：`engine_manager.reconcile(&resolved);` → `engine_manager.defer_reconcile(&resolved);`

### 行为不变
- 首次 webfetch/websearch 调用拿到的 client 与今日启动时建的完全同参（同一 `warmup()`）；未配置 websearch 的报错文案不变；禁用时（不在 desired）仍得 `not warmed`，与今日 `stop_engine` 后一致。
- 首次调用多付一次 client 构建（WP-154 §5 实测 0.4 ms）。

### 测试（Tools 落地）
| 测试名 | 断言 |
|---|---|
| `optional::tests::defer_reconcile_builds_no_client` | `defer_reconcile` 后 `webfetch_client().read().is_none()` 且 `state("webfetch") == None` |
| `optional::tests::ensure_warm_builds_once_for_desired` | `ensure_warm("webfetch")` 两次（含 2 线程并发）后 client `Some`，`state == Warm`，warmup 计数 = 1 |
| `optional::tests::ensure_warm_skips_undesired` | 未启用 websearch → `ensure_warm("websearch")` 后 client 仍 `None` |
| `tools::webfetch::tests::cold_engine_warms_on_first_call` | 只 `defer_reconcile`，调用 fetch 非法 URL → 报错不含 `not warmed`（证明已按需建） |
| `tools::websearch::tests::unconfigured_message_unchanged`（沿用 websearch.rs:131/144 断言） | 未配 endpoint → 仍含 `not configured` |
| 既有 `optional::tests::reconcile_starts_warmup_for_network_core`（mod.rs:300） | 不改，必须仍绿 |
| `tests/startup_order_death_list.rs::serve_run_does_not_warm_optional_engines` | `serve::run` 体内不含 `engine_manager.reconcile(` |

---

## 3. remove_legacy_text_index：P0 → P2（我域）

### 现状
`PathIndex::attach`（src/engines/path_index/mod.rs:83-118）在 :98 同步 `remove_legacy_text_index(&root)`（:400，一次性 `remove_dir_all(.litecode/text-index)`），由 serve/run.rs:86-88 在 READY 前调用。

### 改动
- 删除 :98 这一行；在 worker `run()`（:229）里 **首次 `fill(&root)` 完成且未 stop 之后**、`flush_pending()` 之前调用 `remove_legacy_text_index(&root)`。即：它不再挡 attach/READY，也不和首屏要用的路径补全抢 IO（排在 fill 后 = 「可延后」的 P2 位置）。
- serve/run.rs:86-88 **不改**（attach 仍在原位，只是变快）；无跨域。

### 行为不变
- 目录仍会被删、日志文案不变；`.litecode/**` 本就被 Explorer 预设排除（src/workspace/filter/defaults.rs:38/46），fill 看不到它，先后顺序不影响路径索引结果。
- 换根（attach 新 root）仍会对新 root 执行一次，与今日一致；同 root 重复 attach 且 worker 存活时今日即不执行（:94 早返回），今后同。worker 已结束再 attach 会重跑 run → 同今日。

### 测试
| 测试名 | 断言 |
|---|---|
| 改 `attach_fills_explorer_corpus_and_second_attach_does_not_restart`（mod.rs:612） | 原「attach 返回即 `!legacy.exists()`」改为：在既有 2 s deadline 循环内等到 `app.rs` 可搜后断言 `!legacy.exists()`；其余断言不变 |
| 新 `legacy_text_index_removed_by_worker_after_fill` | 预置 legacy 目录 + 若干文件；attach；deadline 内 legacy 消失且 `mention_paths` 正常 |
| 新 `attach_does_not_touch_legacy_text_index_inline` | 源码扫描：`fn attach` 体内无 `remove_legacy_text_index(`，`fn run` 体内有（death_list 写法） |

---

## 4. LSP `--version` / `npm config get prefix` 探测：P1 → P2 按需（语言通用）

### 现状（调用链）
- `WorkspaceEngines::start("lsp")`（src/engines/mod.rs:1105）起线程跑 `LspEngine::warmup`（src/engines/lsp_engine.rs:48-76）→ `LspHub::activate`（src/lsp/hub.rs:202-218）对 **每个配置的 server** 调 `deps::command_runnable_command_in(cmd, workspace_root)`（deps.rs:490）：`resolve_server_binary` → `resolve_path_candidate` → `npm_bin_search_dirs` → `npm_config_prefix`（deps.rs:129，起 node 进程），再 `run_version_probe`（deps.rs:423，`<server> --version`，cwd=工作区）。任一失败 → warmup `Err("language server '<program>' not runnable: …")` → `EngineState::Failed` + `last_error`。
- lsp_engine.rs:73 注释写「activate is metadata-only」，与事实不符（WP-154 已指出）。
- 依赖该结果的地方：
  - `LspTool::ensure_ready/availability_failure`（src/tools/lsp.rs:56-145）：Warming 时等待、Failed 时返回 `"LSP is unavailable … : <last_error>. Check Settings → Engines → LSP"`。
  - `lsp_feedback`（src/tools/lsp_feedback.rs:42）：非 Warm 则静默。
  - `/api/workspace/engines`（src/workspace/routes.rs:302，summary：state/error）与 `/engines/detail`（routes.rs:319 → status_view.rs:19-45，另跑 `probe_workspace_servers` 得 `usable`）。前端：`engineStore.ts:84-87` 存 lsp state/error；渲染只用 `desired`（EditorPane.tsx:111）与 detail 的 `usable`（EnginesSection.tsx:75）。
- **可见性**：lsp 工具 = bind + `workspace_readiness_from_engines`（src/config/workspace.rs:324，只看 engines.json `desired && servers 非空`），与引擎状态、探测结果无关 —— 符合 Andy「只用 bind 控制可见性」，本包**不改**可见性代码。

### 参考做法（已从 GitHub 拉源码核对，非凭印象）
- **Zed**（zed-industries/zed，main @ e463c5a）：
  - 打开 buffer 才注册并启动 server：`LocalLspStore::register_buffer_with_language_servers`（crates/project/src/lsp_store.rs:3162）→ 按需 `self.start_language_server(…)`（:448，已有实例则复用 :433-435）。
  - 二进制在启动时才解析：`start_language_server`（:477）→ `get_language_server_binary`（:542 调用，定义 :753）→ `CachedLspAdapter::get_language_server_command`（crates/language/src/language.rs:388，带 `cached_binary` 缓存）→ 适配器 `check_if_user_installed`（language.rs:713 trait 默认、:837 调用）。没有单独的打开时可用性探测。
  - 启动前门闩：`get_language_server_binary` 接收 `wait_until_worktree_trust: Option<watch::Receiver<bool>>`（lsp_store.rs:761/769-780），**在 spawn 前 await 一个 watch 门闩**——第 1 项 SweepGate 的 `watch` 形态即仿此。
- **VS Code**（microsoft/vscode，main @ 5ea3f38）：
  - 语言扩展在用到该语言时才激活：`LanguageService` 在 `onDidRequestRichLanguageFeatures` 时 `activateByEvent('onLanguage:${languageId}')`（src/vs/workbench/services/language/common/languageService.ts:286-291）；`AbstractExtensionService.activateByEvent`（src/vs/workbench/services/extensions/common/abstractExtensionService.ts:993）。例：extensions/typescript-language-features/package.json:61 `"onLanguage:typescript"`。
  - 激活后的 LanguageClient 直接 spawn server，失败即报错，不先跑 `--version`：vscode-languageserver-node `client/src/node/main.ts:294 createMessageTransports` → `cp.spawn(command.command, args, options)`（:506/516/530）。
- 采纳：Zed 的「首次需要时解析+校验并缓存」（语言无关，挂在唯一启动点上）+ watch 门闩；不照搬 VS Code 的「跳过校验直接 spawn」，因为我们要保留今日 Failed 文案。

### 改动（我域为主）
1. **src/engines/lsp_engine.rs:48-76 `warmup`**：仍做 engines.json 读取、`commands_for_server_ids` 与两条空配置报错（文案不变）；`self.hub.activate(&commands)` 改为 `self.hub.activate_deferred(&commands)`（只记 `configured_commands`、`active=true`、清探测缓存）。warmup 因而毫秒级返回 → `Warm`。修正 :73 注释。
2. **src/lsp/hub.rs:202 `activate` 拆分**：
   - `activate_deferred(commands)`：上述元数据；`probe: tokio::sync::OnceCell<Result<(), String>>` 重置（随 `stop()` 与新 activate 一起换新 cell，即按 warmup epoch 失效）。
   - `pub async fn ensure_probed(&self) -> Result<(), String>`：`probe.get_or_init` 里 `spawn_blocking` 跑**原 activate 循环原样**（全部 configured commands、同一 `command_runnable_command_in(cmd, workspace_root)`、同一错误串 `language server '{program}' not runnable: {e}`）。全量探测而非只探目标语言，保证「一个 server 坏 → 整个 lsp Failed」的今日语义不变；结果缓存，每 epoch 至多一次。
   - 失败回写：hub 持 `probe_failed: Option<Arc<dyn Fn(String) + Send + Sync>>`，由 `WorkspaceEngines::new`（src/engines/mod.rs）注入，回调里对 `states["lsp"]` 写 `Failed`、`last_errors["lsp"]` 写原错误串（若当前为 `Stopped` 则不写，与 mod.rs `finish` 闭包同规则）。**只用既有状态**。
   - `resolve_server_key`（hub.rs:220）在「active 校验」后、解析/spawn 前 `self.ensure_probed().await.map_err(LitecodeError::Config)?`。→ 编辑器 RPC、`lsp_feedback`、`restart_server`（:384）全部覆盖。
   - `npm config get prefix` 随 `resolve_server_binary` 一起只在探测/spawn 时发生（与 Zed `get_language_server_binary` 同时机）；本包不另加缓存，避免改变 PATH/npm 变更后的解析结果。
3. **src/engines/mod.rs** 新增 `pub async fn ensure_lsp_probed(&self) -> Result<(), String>`（转调 hub；`state("lsp") != Warm` 时直接返回 Ok 不探测）及同步版 `ensure_lsp_probed_blocking`（给 spawn_blocking 上下文用）。
4. **src/engines/status_view.rs:19 `engines_detail_view`**：开头调 `ensure_lsp_probed_blocking()`，使 detail（设置页 Engines）读到的 `state/error/usable` 与今日完全一致（该端点本就会跑 `probe_workspace_servers`，属按需）。`lsp_usability`（:47）同理。
5. **跨域（Tools）src/tools/lsp.rs:103 `ensure_ready`**：`if self.is_ready()` 分支前加
   `if let Err(_) = self.engines.ensure_lsp_probed().await { /* state 已被回写为 Failed */ }`，随后走原逻辑 → Failed 分支返回原 `availability_failure` 文案。（+3 行，不改文案、不改等待逻辑）
   `lsp_feedback.rs` 不改：Warm 时调 hub → hub 内 `ensure_probed` 失败 → `Unavailable` → 原样不附提示，与今日 Failed 时静默一致。

### 引擎状态与 UI 的表现（行为对照）
| 场景 | 今日 | 改后 |
|---|---|---|
| 所有 server 可运行 | Warming(~0.4–0.6 s 探测) → Warm | Warming(ms) → Warm；首次需要时后台探测一次，成功无变化 |
| 某 server 不可运行，打开设置 → Engines | state=failed，error=`language server 'X' not runnable: …`，usable=unavailable | detail 请求触发探测 → **相同** state/error/usable |
| 同上，agent 调 lsp 工具 | `LSP is unavailable in the current workspace (root=…): language server 'X' not runnable: …. Check Settings → Engines → LSP, then retry` | ensure_ready 先探测 → 回写 Failed → **同一文案** |
| 同上，写文件后的诊断提示 | 非 Warm → 静默 | hub 探测失败 → Unavailable → 静默 |
| 同上，编辑器打开该语言文件（RPC 经 hub） | activate 失败未置 active → `LSP hub not active` | resolve_server_key 探测失败 → 返回 `language server 'X' not runnable: …`，状态转 Failed。**错误串不同**（更具体），见未决 Q6 |
| `/api/workspace/engines` summary（P0 后前端会拉，WP-168 将挪到 P2） | failed | **首次需要前为 warm**；前端只用其 `desired`（EditorPane.tsx:111），无可见差异 → 见未决 Q1 |
| 工具可见性 | bind + engines.json | 不变 |

### 测试
| 测试名 | 断言 |
|---|---|
| `engines::lsp_engine::tests::warmup_spawns_no_probe_process` | engines.json 配一个 server，命令指向测试脚本（被 `--version` 调用即写 marker）；`warmup()` Ok、`state==Warm` 后 marker 不存在 |
| `engines::tests::lsp_first_need_probe_failure_sets_failed_same_error` | 配不存在的命令；warmup 后 Warm；`ensure_lsp_probed().await` Err；`state("lsp")==Failed`；`last_error` 以 `language server '<program>' not runnable:` 开头（与 hub.rs:208 今日文案逐字一致） |
| `engines::tests::lsp_probe_runs_once_per_epoch` | 计数脚本：两次 ensure + 一次 resolve → 计数 1；`stop()`+重新 start 后再 ensure → 计数 2 |
| `engines::tests::lsp_probe_failure_after_stop_does_not_resurrect_state` | 探测进行中 stop → 回调不覆盖 `Stopped` |
| `engines::status_view::tests::detail_after_deferred_probe_matches_eager` | 坏命令：`engines_detail_view` 的 `lsp.state=="failed"`、`error` 同上、`usable=="unavailable"` |
| `lsp::hub::tests::resolve_server_key_probes_before_spawn` | 坏命令下 resolve 返回含 `not runnable` 的错误且 mock spawn 未被调用 |
| 既有 `lsp::mod::tests::coverage_requires_active_and_configured_command`、`stop_clears_active_and_coverage`、`restart_server_rejects_*` | 不改，必须仍绿（activate_deferred 维持 active/configured 语义） |
| Tools：`tools::lsp::tests::broken_server_first_call_message_unchanged` | 坏命令 + Warm 起步；首次工具调用返回文本 == 今日 Failed 分支 `availability_failure` 文本 |
| `tests/startup_order_death_list.rs::lsp_warmup_runs_no_subprocess` | `LspEngine::warmup` 体内不含 `activate(`/`command_runnable_command`；`activate_deferred` 体内不含 `run_version_probe`/`npm_`/`Command::new` |
| `…::no_language_specific_branch_added` | 新增函数（`activate_deferred`、`ensure_probed`、`ensure_lsp_probed*`、`sweep_gate` 模块）体内不出现 `SERVER_METAS` 中任一 server id 字面量（既有 deps.rs:494 pyright 分支原样保留，不在扫描范围） |

---

## 5. 跨域 diff，交 owner 落地（尽量只移动调用）
| owner | 文件:行 | 改动 | 规模 |
|---|---|---|---|
| session | src/serve/run.rs:73-75 | `sweep_orphan_lsp_processes()` → `crate::lsp::sweep_gate::global().start_background(crate::proc_lifetime::sweep_orphan_lsp_processes)` | 1 行替换 |
| session | src/serve/run.rs:84 | `engine_manager.reconcile(&resolved)` → `engine_manager.defer_reconcile(&resolved)` | 1 行替换 |
| tools | src/optional/mod.rs:149 附近 | 新增 `defer_reconcile` / `ensure_warm` / `webfetch_ensure` / `websearch_ensure`；`start_warmup` 同步分支抽 `warm_now` | ~40 行，原逻辑不改 |
| tools | src/tool/registry.rs:130/134 | 构造工具时多传 ensure 回调 | 2 处 |
| tools | src/tools/webfetch.rs:22、websearch.rs:25 | client 为 None 时先 ensure 再读 | 各 +2 行 |
| tools | src/tools/lsp.rs:103 | `ensure_ready` 开头 `ensure_lsp_probed().await` | +3 行 |
我将把以上写成可 `git apply --check` 的 diff 放 `.litecode/ops/artifacts/WP-167/cross-domain/{session,tools}.diff`（批准后、与我域 diff 一起），由 owner 审后落地。

## 6. 编译与测试计划
- `target/` 已不存在（回滚后），**首轮必须全量重编**；重型任务一律等 backend_mid 串行放行（WP-165 流程：单重型 ≥4.5 GiB）。
- 环境：`HOME=.tmp/home`、`timeout`、`</dev/null`、共享 target 排队；不 commit / checkout / stash。
- 顺序：① `cargo test --lib lsp::sweep_gate lsp::hub engines::lsp_engine engines::path_index engines::status_view engines::tests` ② `--test startup_order_death_list` ③ Tools 落地后 `--lib optional tools::webfetch tools::websearch tools::lsp` ④ 全量 `cargo test`（含 `--test optional_code_search`、`code_search_ipc`）⑤ 启动时序复测：沿用 WP-155 harness（`.litecode/ops/artifacts/WP-155/harness`）各跑 3 次，确认 READY 前不再出现 sweep/HTTP client/legacy 删除，且打开后无 `npm`/`--version` 子进程直到首次 LSP 需要。

## 7. 未决问题（请 backend_mid 定）
1. **summary 端点的 lsp.state**：坏 server 在「首次需要」前读作 `warm`（今日 `failed`）。前端不渲染它（只用 desired），设置页走 detail 已一致。可接受？若不可，备选是让 `/engines` summary 也触发探测——但 WP-168 挪到 P2 之前它在启动即被调用，会把探测拉回启动期。
2. **webfetch/websearch 按需的形态**由 Tools 决定：上表回调式是我建议的最小改动；若 Tools 更愿意让工具直接持 `Arc<EngineManager>` 也可，只要求 serve::run 不再同步建 client。
3. **src/proc_lifetime.rs 无 domain 归属**：本方案不改它；若要把门闩放进它（更内聚），需先定 owner。
4. **sweep 门闩不设超时**：保证优先；若希望兜底（例如 10 s 后放行并 warn），会弱化「必先完成」，需 GM 口径。
5. lsp 工具 Warming 等待（tools/lsp.rs `LSP_WARM_WAIT`）在改后几乎不会触发（warmup 变毫秒级），首次探测改在 ensure 内 await；工具首调的总等待不超过今日（今日等 warmup 探测，改后等同一探测）。仅告知。
6. **编辑器 RPC 的错误串**：坏 server 时今日为 `LSP hub not active`，改后为 `language server 'X' not runnable: …`。如要逐字不变，可在 hub 探测失败后同时置 `active=false` 并返回原串（代价：一个语言坏会让后续请求都走 inactive，与今日一致）。倾向后者以守「行为不变」，请定。

---

## 8. 批准后的裁定与实施记录（2026-10-15 02:36 批，ide 实施）
裁定（backend_mid）：
1. **接受。** 坏 server 在首次需要前，`/api/workspace/engines` 摘要中 lsp.state 读作 `warm`。**前端只读该响应的 `desired`**（EditorPane.tsx:111；engineStore 存 state/error 但无渲染）；**详情视图 `/engines/detail` 与现状一致**（`engines_detail_view` 开头触发探测，state/error/usable 同今日）。
2. web 工具触发预热的形态由 Tools 定，tools.diff 需 Tools 签字；当前 tools.diff 为我按方案写的一版，**形状待 Tools 定**。
3. 门闩放 src/lsp（`src/lsp/sweep_gate.rs`），不动 proc_lifetime.rs。
4. 门闩不设超时（death list `sweep_gate_has_no_timeout` 锁定）。
5. 探测失败时 hub 置 inactive，编辑器/重启路径返回原文本 `LSP hub not active`（`resolve_server_key`、`restart_server`）；engine 记 `Failed` + 原 `config error: language server '<program>' not runnable: …`。

产物：`ide.diff`（我域，git diff HEAD + 新文件 `git diff --no-index /dev/null`，未动 index），`cross-domain/session.diff`（基于当前 HEAD 的 run.rs，WP-166 尚未改 run.rs，**待 rebase 到 WP-166 之上**），`cross-domain/tools.diff`（待 Tools 签字）。

与 §4 测试表的偏差：
- `warmup_spawns_no_probe_process` 未写成 LspEngine 级测试（需改全局 env `LITECODE_LSP_SERVERS`，会与并行单测互相干扰）；由 hub 级 `deferred_activate_runs_no_probe_until_first_need` + death list `lsp_warmup_runs_no_subprocess` 覆盖。
- `tools::lsp::tests::broken_server_first_call_message_unchanged` 属 Tools，未写；engine 级 `lsp_first_need_probe_failure_sets_failed_same_error` 锁定了 Failed + 原错误串，tools 的 `availability_failure` 文案不变。
- `tools::webfetch::tests::cold_engine_warms_on_first_call` 由 optional 级 `ensure_warm_builds_once_on_first_use` 代替（不发网络请求）。
- 已知窄窗：warmup 置 active 到 finish 写 Warm 之间（微秒级）若有请求完成整轮探测（数百 ms）并失败，Failed 可能被随后的 Warm 覆盖；实际不可达，记录在案。

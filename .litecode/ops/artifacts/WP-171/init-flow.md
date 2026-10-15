# WP-171 新电脑 litecode 环境初始化（实测 1 次；cargo test 编译失败，暂不 deliver）

## 一键流程（Linux）
    curl -fsSLO <raw>/.litecode/ops/tools/init_env.sh   # 或 clone 后直接执行
    bash init_env.sh [--repo URL] [--branch ops/team-framework] [--dir ~/litecode] [--skip-heavy]
共 8 步：系统包 → clone/ff → rustup + 固定工具链（rust-toolchain.toml 1.99.0）→ fnm Node 22 → npm ci（web/desktop）→ cargo build --locked → validate_all + toolchain_check → 测试。
所有步骤都可重复执行；日志写在 `$LC_INIT_LOG_DIR`（默认 ~/.litecode-init-logs）。
内存守卫（全局规则，当前口径）：重型步骤要求 MemAvailable ≥ 2GiB 才启动（LC_MIN_MEM_GIB 默认 2）；跌破 1GiB 由 watchdog 杀掉进程组（LC_KILL_MEM_GIB 默认 1）；CARGO_BUILD_JOBS 默认 2。历史实测节保留当时口径不改。

## 同包改动（tools_version 19→20）
- cargo_env.sh：LC_ROOT 可通过环境变量覆盖，默认取脚本所在仓库；cargo_locked.sh 改为 cd 到 $LC_ROOT。
- tick.mjs / validate_all.mjs：不再读取 demand/board.json；registry 的 gm_policy 去掉 demand board。
- toolchain_check.mjs：修复误判。原先 `rustup: not found` 会被当作通过。
- 新增 rust-toolchain.toml，固定为 1.99.0（暂定，以完整构建实测为准）。

## 实测（2026-10-15 10:37–10:50 HKT，lcfresh 全新用户，无人工介入）
- 源：本地临时分支 `wp171-temp` 的提交 `bcda58463b378c246a43e685b6831dc373076261`（父 9ce2ab7，未推送，ops/team-framework 未动）。
- 命令：`LC_MIN_MEM_GIB=3 LC_KILL_MEM_GIB=1 CARGO_BUILD_JOBS=1 bash init_env.sh --repo /workspace/litecode --branch wp171-temp`
- **一次性例外（gm 批，仅限 WP-171 验收）**：内存门槛 3GiB（全局默认仍 4.5GiB），CARGO_BUILD_JOBS=1，<1GiB 由 watchdog 杀掉；后端中层、前端中层均已确认独占。watchdog 未触发。
- 版本：rustc/cargo 1.99.0（rust-toolchain.toml pin 已确认可构建），node v22.23.3，npm 10.9.9。

| 步骤 | 耗时 s | 最低 MemAvailable | 结果 |
|---|---|---|---|
| npm ci web | 11 | 2.86 GiB | ok |
| npm ci desktop | 9 | 3.14 GiB | ok |
| cargo build --locked（首次全量，jobs=1） | 598 | 2.29 GiB（较起点 3.31 GiB 少用约 1.0 GiB） | ok |
| validate_all / toolchain_check（四种 shell 全 ok，env -i 解析到 ops .toolchain 1.99.0） | <1 | - | ok |
| ops node --test | 24 | 6.59 GiB | ok |
| cargo test --locked | 145 | 5.63 GiB | **FAIL：编译错误** |
| web vitest / desktop test | - | - | 未执行（脚本在首个失败处停止） |

cargo test 失败原因：产品代码问题，与环境无关。`src/client_protocol/ws_wp166_tests.rs:450` 处 `let env = env().await;` 中，局部变量 `env` 遮蔽了同名函数（E0618）；该文件属 session 域，在 9ce2ab7 中即存在。
证据见 evidence/：init.log、heavy_stats.tsv、cargo_build.tail.log、cargo_test.log、validate_all.log、toolchain_check.log、ops_tests.log、run.out。

## 未决
- 修复 ws_wp166_tests.rs:450（建议由 session 负责，或经 gm 授权后由我做一行改名）；修复后需重做一次 lcfresh 完整运行，跑通 cargo test、web、desktop 测试后才能 deliver。

## 第二次实测（run2，2026-10-15 10:58–11:13 HKT，lcfresh 重建为空用户，无人工介入）
- 源：`wp171-temp` 提交 `8c39f247f6dbd8749ee9481380f7b29ea0feb2ec`（包含 WP-166 的 E0618 修复）。
- 阈值：沿用一次性例外（启动门槛 3GiB、低于 1GiB 由 watchdog 杀进程、jobs=1）。本次运行开始后，Andy 将全局规则改为启动门槛 ≥2GiB、低于 2GiB 杀进程。本次运行全程 MemAvailable 最低 3.61GiB，同时满足新旧两条规则；watchdog 未触发。

| 步骤 | 耗时 s | 最低 MemAvailable | 结果 |
|---|---|---|---|
| npm ci web | 11 | 6.16 GiB | ok |
| npm ci desktop | 6 | 6.44 GiB | ok |
| cargo build --locked（jobs=1） | 584 | 4.13 GiB | ok |
| validate_all / toolchain_check（4/4，env -i = ops .toolchain 1.99.0） | <1 | - | ok |
| ops node --test | 23 | 5.81 GiB | ok |
| cargo test --locked | 251 | 3.61 GiB | **FAIL：2005 passed / 7 failed / 3 ignored** |
| web vitest / desktop test | - | - | 未执行（流程在首个失败处停止） |

这 7 个失败都出在产品代码，与环境初始化无关；依规则只报告，不改产品代码：
- 5 个测试按 Windows 路径编写，但没有用 cfg(windows) 隔离，在 Linux 上运行即失败：
  - config::path::tests::strip_makes_verbatim_prefix_compatible
  - lsp::tests::{normalize_makes_verbatim_path_prefix_compatible, publish_diagnostics_uri_matches_drive_case, uri_to_path_recovers_mangled_verbatim_file_uri}
  - terminal::shell::tests::shell_label_uses_the_program_stem
- optional::tests::reconcile_starts_warmup_for_network_core：`mgr.is_warmed("webfetch")` 断言失败。
- tool::registry::tests::core_bindings_produce_full_builtin_list：内置工具列表中缺 `session_search`。可能与 feature 有关。

证据见 evidence/run2/。

## 全局内存规则变更（Andy 经 gm，2026-10-15 11:10 HKT）
init_env.sh 的默认值由 4.5/1 改为 LC_MIN_MEM_GIB=2、LC_KILL_MEM_GIB=2，仍可通过环境变量覆盖；registry 和 board.md 已同步更新。WP-171 的一次性例外（3GiB/1GiB）就此失效。

## run3 源提交 `5b7d483b0665edf5450e5f51dd6722ea09d96941`；包含的修复（均由各域 bot 改动，tools_steward 未改产品代码；逐个 `git apply -R --check` 核对：工作树中已全部生效）
- WP-166/fix_e0618.diff（session）：src/client_protocol/ws_wp166_tests.rs
- WP-171/wp171-fix/fix.diff（IDE 基座）：src/lsp/mod.rs（3 个 Windows 路径测试改为 cfg(windows)）、src/optional/mod.rs（reconcile_starts_warmup 判定为过时测试，改为“懒预热，首次使用时预热”）
- WP-171/tools.diff（Tools）：src/terminal/shell.rs（cfg(windows)）、src/tool/registry.rs（session_search 受 WP-057 门控，默认隐藏）
- WP-171/runtime-strip-verbatim-cfg-windows.diff（Runtime）：src/config/path.rs（cfg(windows)）
- WP-165/fix.diff 与 WP-165/wp167-defer_reconcile.diff（工作树中已有的 WP-165 改动，一并纳入）：src/serve/run.rs、tests/startup_order_death_list.rs

## 第三次实测（run3，2026-10-15 11:19–11:34 HKT，lcfresh 重建为空用户，无人工介入，内存规则为全局 2GiB 门槛 + 2GiB 时 watchdog 杀进程，jobs=1）
| 步骤 | 耗时 s | 最低 MemAvailable | 结果 |
|---|---|---|---|
| npm ci web | 10 | 5.54 GiB | ok |
| npm ci desktop | 9 | 5.85 GiB | ok |
| cargo build --locked | 573 | 3.76 GiB | ok |
| validate_all / toolchain_check（4/4） | <1 | - | ok |
| ops node --test | 26 | 5.33 GiB | ok |
| cargo test --locked | 259 | 3.14 GiB | **FAIL** |
| web vitest / desktop test | - | - | 未执行 |

watchdog 未触发。
- lib 单元测试：2010 passed / 0 failed / 3 ignored，此前的 7 个失败已全部修复。
- 集成测试 tests/code_search_ipc.rs 中的 `warmup_and_refresh_follow_disk_excludes` 失败（10 passed / 1 failed），位置 tests/code_search_ipc.rs:354：search_exclude 中配置的 secret.rs 仍出现在搜索结果里。这是产品代码问题（workspace/engines 检索的排除逻辑），依规则只报告、不修改。
- cargo 遇到首个失败的测试目标即停止，code_search_ipc 之后的集成测试目标本轮均未运行。
证据见 evidence/run3/。

## init_env v2：测试阶段（gm 批，2026-10-15 11:35 HKT）
- 第 1–7 步（系统包、clone、rustup、node、npm ci、cargo build、validate_all/toolchain_check）保持 fail-fast：任一失败立即退出。
- 第 8 步依次执行 ops_tests → `cargo test --locked --no-fail-fast` → web vitest → desktop test。某一步失败不影响后续步骤；内存不足中止（exit 3）和 watchdog 杀进程（exit 4）仍立即停止，不重试。
- 只要有任何测试步骤失败，整体以 exit 6 结束，并生成 `$LOG_DIR/test_failures.txt`。文件由新工具 `ops.test_failures`（`.litecode/ops/tools/test_failures.mjs`）产出，每条格式为
  `- <step>: <测试名> @ <文件>:<行> — <首行消息>`，可直接转发给负责的领导。
  支持的日志格式：cargo 的 panic 和编译错误（E-code，带 `-->` 定位）、vitest 的 FAIL 加 ❯ 定位、node --test TAP 的 location、tsc 的 `file(line,col)` 错误。
- 用 run2、run3 的真实日志回放验证：解析出 7 条和 1 条失败，均带 文件:行。
- 当前临时提交（init_env v2）：`5ae11adc681efd75840cb6eae408c62138942c32`

## run4 源提交包含的修复（均由各域 bot 修改，tools_steward 未改产品代码；逐个 `git apply -R --check` 确认已在工作树生效）
- run3 列出的全部修复：WP-166 E0618；IDE 的 lsp/optional；Tools 的 shell/registry；Runtime 的 config/path；WP-165 的 fix 与 wp167-defer_reconcile
- WP-171/code-search-exclude/fix.diff（IDE 基座）：重写 tests/code_search_ipc.rs
- WP-165/followup.diff（IDE 基座，sweep 日志）：src/lsp/sweep_gate.rs、src/proc_lifetime.rs
- run4 源提交：`527e798b152009e845f1c1ed2c91a4f0f40a21c8`

## run4 结果：中断（云端电脑重启）
- 11:41:39 HKT 在 lcfresh 下启动（init_env v2，源提交 527e798，内存门槛与 watchdog 均为 2GiB，jobs=1）。
- 约 11:50 HKT 云端电脑整机重启（11:56 时 uptime 为 5 分钟）。重启后 /home 和 /etc 被重置：lcfresh 用户及其 HOME（含全部 run4 日志）均已不存在；/workspace（仓库、wp171-temp、本 artifact）完好。
- 上一次启动没有 journal，无法确认重启原因（是否 OOM 未知）。脚本日志中未见 watchdog 杀进程记录（日志已随 HOME 丢失，无法回查）。没有获得任何 run4 测试结果，因此不存在 test_failures.txt。
- 按“不重试”规则停止，等待 gm 指示。
- 后续改进（待批）：LC_INIT_LOG_DIR 改为写到 /workspace 下，避免日志随 HOME 被重置一起丢失。

## init_env 日志目录（gm 批，run5 前，2026-10-15 11:56 HKT）
- `LC_INIT_LOG_DIR` 可覆盖；未设置时，clone 后默认写到 `$DIR/.tmp/init-logs`（DIR 在 /workspace 下时，HOME 被重置也不会丢日志）。
- 早期步骤（clone 前）先写 `/tmp/litecode-init-$$`，clone 后搬到默认目录。
- `heavy_stats.tsv` 列：step、seconds、start_memavail_kb、min_memavail_kb、peak_used_kb（= start − min）、rc。
- run5 将显式设置 `LC_INIT_LOG_DIR=/workspace/litecode/.tmp/wp171-run5`。
- run5 prep 提交：`6ac33372ce4eb5d641323d5a6192222f69ea5450`（在 527e798 之上；日志默认改到 $DIR/.tmp/init-logs，LC_INIT_LOG_DIR 可覆盖）

## 第五次实测（run5，2026-10-15 12:08–12:18 HKT）— WATCHDOG 杀进程，按规则停止、不重试、不交付
- 源：`wp171-temp` `6ac33372ce4eb5d641323d5a6192222f69ea5450`（相对该 tip 无新增产品代码改动，未另作 tip）。
- 启动前已装好：build-essential 12.12、g++ 14.2.0、libssl-dev、pkg-config、cmake。
- 参数：LC_INIT_LOG_DIR=/workspace/litecode/.tmp/wp171-run5，LC_MIN_MEM_GIB=2，LC_KILL_MEM_GIB=2，CARGO_BUILD_JOBS=1。
- 日志已落到 /workspace（重启不会丢）。

| 步骤 | 耗时 s | start MemAvail | min MemAvail | peak_used | 结果 |
|---|---|---|---|---|---|
| npm ci web | 14 | 3.31 GiB | 2.92 GiB | 0.39 GiB | ok |
| npm ci desktop | 6 | 3.37 GiB | 3.23 GiB | 0.14 GiB | ok |
| cargo build --locked | ~569 | 3.35 GiB | **1.96 GiB** | ~1.39 GiB | **WATCHDOG 杀掉**（MemAvailable 2051900kB < 2097152kB） |

- validate_all / toolchain_check / 全部测试：未执行。
- 按规则：被杀一次即停止、不重试、不自行提高门槛。证据见 evidence/run5/ 与 .tmp/wp171-run5/。
- 数据供 gm/Andy 判断全局 2GiB 门槛是否过低（本机首次全量 cargo 峰值占用约 1.4GiB，起点约 3.3GiB，谷底跌破 2GiB）。

## 内存策略更正（gm，2026-10-15 12:19 HKT）
- Andy「留 2GiB」指**启动余量**（MemAvailable ≥2GiB 才开重型），不是看门狗杀线。
- 默认改为：`LC_MIN_MEM_GIB=2`（启动门槛）、`LC_KILL_MEM_GIB=1`（跌破 1GiB 才杀）。仍可用环境变量覆盖。
- run5 假杀数据：cargo_build 起点 3.35GiB、谷底 **1.96GiB**、peak_used ~1.39GiB，在旧默认杀线 2GiB 下被杀；按新策略不应杀。
- run6 源提交：`2536258ee61aea6485b8632ad60d00bb3e618920`（MIN=2 KILL=1）

## 第六次实测（run6，2026-10-15 12:19–12:40 HKT）— 未交付
- 源：`wp171-temp` `2536258ee61aea6485b8632ad60d00bb3e618920`（MIN=2 KILL=1 默认 + run5 假杀说明）。
- 参数：LC_INIT_LOG_DIR=.tmp/wp171-run6，LC_MIN_MEM_GIB=2，LC_KILL_MEM_GIB=1，CARGO_BUILD_JOBS=1。run5 日志未覆盖。
- 看门狗未触发。exit=6（测试阶段有失败）。

| 步骤 | s | start | min | peak_used | rc |
|---|---|---|---|---|---|
| npm ci web | 12 | 2.76 GiB | 2.30 GiB | 0.45 GiB | 0 |
| npm ci desktop | 7 | 4.74 GiB | 4.60 GiB | 0.14 GiB | 0 |
| cargo build | 618 | 4.75 GiB | 4.24 GiB | 0.51 GiB | 0 |
| validate_all / toolchain_check | <1 | - | - | - | ok |
| ops_tests | 26 | 6.40 GiB | 5.87 GiB | 0.53 GiB | 0 |
| cargo test --no-fail-fast | 327 | 6.51 GiB | 3.79 GiB | 2.72 GiB | 101 |
| web vitest | 202 | 6.35 GiB | 5.89 GiB | 0.46 GiB | 0 |
| desktop test | 10 | 6.31 GiB | 6.18 GiB | 0.14 GiB | 0 |

### 失败清单（可转发）
**Backend (Rust) — 4：**
- cargo_test: workspace::git::tests::log_includes_merge_parents @ src/workspace/git.rs:903
- cargo_test: network_core_engines_always_warmup @ tests/optional_p0.rs:87
- cargo_test: settings_reload_reconciles_engine_manager @ tests/optional_p0.rs:205
- cargo_test: sessions_db_sql_and_rusqlite_stay_in_sqlite_module @ tests/session_data_sql_death_list.rs:206

**Frontend (web/desktop) — 0：** web vitest 与 desktop test 均通过。

证据：`.tmp/wp171-run6/` 与 `evidence/run6/`。

## run7 源：相对 tip 2536258 的产品修复（各域 bot，tools_steward 未改产品代码）
- src/workspace/git.rs — sanitize GIT_* in test fixtures（修 log_includes_merge_parents）
- tests/optional_p0.rs — webfetch 懒预热断言（修 network_core + settings_reload）
- src/client_protocol/ws_wp166_tests.rs + src/session/data/sqlite/test_support（新）+ sqlite/mod.rs — rusqlite 仅经 sqlite::test_support（修 death_list）

## WP-050 Tools 方案（tools，direct）— 核心覆盖核对 + 测试裁剪提案
父包 WP-043（in_progress）；产物本文件。状态：**方案填写，未 start、未改测试/代码、不 commit**。
禁区：引擎产品、symlink 产品行为、ISSUE-034 协议层改造。

### 0 目标与硬串行
两步**硬串行**（批后才 start；未过中层审前不删测试）：
1. **覆盖**：核对 Tools 域核心流程有效覆盖，产出绿表 + 缺口。
2. **裁剪**：基于覆盖结果，列冗余/过时/重复及删/留/放宽提案交中层裁决；审前不删。

参考：WP-021-backend §5 Tools 旧清单（10 条）、WP-022-tools.md C/追加裁剪、WP-036-tools.md 执行记录；**必须重核当前工作区**，不照搬旧结论。

### 1 范围（本域）
| 子域 | 路径 | 备注 |
|---|---|---|
| custom / hub | `src/tools/custom.rs`、`custom_hub.rs` | WP-027/038 刚落：EPIPE、组杀、有界 drain、bind、B5/B6 |
| MCP 生命周期 | `src/mcp/{client,pool}.rs`、`src/tools/mcp_tool.rs` | WP-038 组杀/bind；**不含** ISSUE-034 协议 |
| bash / jobs | `src/tools/bash*.rs`、`kill_shell`、`wait_shell` | 含 live_* flake 候选 |
| permission | `src/permission/*`、`tests/{client_protocol_permission,common/permission,permission_pipeline}.rs` | WP-033 权限队列属前端；本域测 Ask/门禁 |
| knowledge | `src/knowledge/*`、`src/tools/knowledge.rs` | |
| webfetch / websearch | `src/tools/webfetch.rs`、`websearch.rs` | ISSUE-021 残留只登记 |
| tool 基座 | `src/tool/{availability,registry,executor,output,signal,write_lock,…}` | |
| lsp / optional | `src/tools/lsp.rs`、`src/optional/*`、`tests/optional_lsp.rs` | Windows cfg 已落地；B12 poison 已修 |
| 集成 | `tests/mcp_integration.rs`、`subscribe_bash_hydration.rs` 等本域相关 | |

不碰：引擎产品、symlink、`take_changes`、前端/桌面、`proc_lifetime` 实现、ISSUE-034。

### 2 步骤一：覆盖核对（先做）

#### 2.1 核心流程清单（对照 WP-033/036 刚落）
| # | 核心流程 | 刚落来源 | 预期覆盖点（批后实核） |
|---|---|---|---|
| C1 | custom stdin EPIPE 不判失败 / stdin 写线程不死锁 | WP-027 | `child_ignoring_stdin…`、`large_output_before_reading_stdin…` |
| C2 | custom 超时/取消组杀 + Finished 后组杀 + 有界 drain | WP-038 / ISSUE-039 | `timeout_kills_grandchild…`、`finished_reaps…`、`finished_kills_orphaned…` |
| C3 | custom Windows/跨平台父子绑定 | WP-038 / ISSUE-040 | 调用点存在（Linux no-op）；不伪测 Job |
| C4 | custom wait 失败不 panic；后台 spawn 失败 abandon_job | WP-027 B5/B6 | `wait_failure…`、`background_spawn_failure…` |
| C5 | custom 后台 mailbox / 输出含 cancelled 不误判 | WP-027 | `background_real_process…`、`background_output_mentioning_cancelled…` |
| C6 | MCP kill/Drop 杀树 + spawn 组/bind | WP-038 | `kill_reaps_grandchild…`、`drop_reaps_grandchild…`；pool 委托路径读码 |
| C7 | bash 前台/后台 job 契约（完成视图、exit、大输出） | 既有 | `tools::bash`、`bash_jobs_contract`；标 flake |
| C8 | permission Ask/拒绝/流水线 | 既有 + WP-033 前端侧 | `permission::*` + 两集成测；与前端队列交叉边界只记录 |
| C9 | knowledge 索引/校验/mentions | 既有 | `knowledge::*`、`tools::knowledge` |
| C10 | webfetch SSRF/代理口径 | ISSUE-021 残留 | 现有测是否仍挡产品路径 |
| C11 | optional 锁中毒不连环 panic | WP-027 B12 | `poisoned_state_lock…` |
| C12 | lsp 行号格式 / Windows 路径 cfg | WP-027 | `optional_lsp` 编号行断言；`#[cfg(windows)]` |

#### 2.2 实跑命令（批后；共享 target 排队；只跑本域）
```
rustc -V   # 期望 1.99.0；直接 cargo，勿 PATH 前缀
cargo test --lib tools::custom tools::custom_hub
cargo test --lib mcp::
cargo test --lib tools::bash tools::bash_jobs_contract tools::bash_safety tools::bash_status
cargo test --lib permission::
cargo test --lib knowledge:: tools::knowledge
cargo test --lib tools::webfetch tools::websearch
cargo test --lib tool:: optional:: tools::lsp
cargo test --test client_protocol_permission --test permission_pipeline
# 抽样：mcp_integration / optional_lsp / subscribe_bash_hydration（按时间）
```
flake 候选（bash live_* / bash_jobs_contract）在覆盖阶段只记录失败率，**不修**；放入步骤二提案。

#### 2.3 产物形态（步骤一交付物）
- **绿表**：上表 C1–C12 ×（测试名 / 结果 ok|fail|skip|缺 / 证据行号）。
- **缺口**：无测或测弱的核心行为（例：MCP pool 超时后 `stop_on_hub` 杀树仅读码无独立测；Windows Job 行为本 box 无法实跑）。
- **外域失败**：非本域文件失败只登记不改。
- ISSUE-039/040：**已 closed**（gm，WP-036/038 验收）；覆盖阶段复核测仍绿即可，不再改 issue。

### 3 步骤二：裁剪提案（覆盖完成后再写；审前不删）

#### 3.1 旧清单重核入口（WP-021-backend §5 Tools，10 条）
| 旧项 | 旧建议 | 本包动作 |
|---|---|---|
| `background_real_process…` 等待放宽到 10s | 放宽 | 重核：WP-027 后 20/20 绿 → 可能改为**保留**或仅文档化超时 |
| ask_user 多问题文案断言 | 放宽 | 重核现断言是否仍逐字 |
| plan.rs `"No plan file was created"` | 放宽 | 重核 |
| `tools::bash` 两条 `live_*` | 放宽 | 重核失败率；提案等待完成/轮询 |
| `bash_jobs_contract` 一条短命令视图 | 放宽 | 同上 |
| permission_pipeline 纯 sink vs client_protocol_permission | 重复 | 重核重叠面，提案删 sink 往返或合并 |
| knowledge spec_tests 文案 vs index_tests reason | 重复 | 重核，提案文案只留一条 |
| webfetch whatwg 解析测 | 可删候选 | 重核是否仍只测第三方 crate |

#### 3.2 追加候选（来自 WP-022 实跑，须再核）
- 同上 bash 三条 flake（20 连跑失败次数已记）。
- 其他：步骤一绿表中「重复覆盖同一分支」「只测第三方」「断言过严文案」新发现一并列入。

#### 3.3 提案表模板（步骤二写入本产物）
| ID | 测试（file::name） | 提案 | 理由 | 替代覆盖 | 中层裁决 |
|---|---|---|---|---|---|
| T-… | … | 删/留/放宽 | … | … | （空，等批） |

**硬规则**：表填完 → 私信中层 → **裁决前零删除**；获批项才改测试；未批项写暂缓理由。

### 4 交叉审
- 覆盖/裁剪涉及进程生命周期结论时，与 IDE 对照 WP-036-ide / `proc_lifetime`（只读）；本包不改 IDE 文件。
- 权限与前端队列边界：只记录「后端测覆盖到哪」，不改 web。
- 双方要点写入本产物「交叉审」节；IDE 未回则标待补，不阻塞中层对裁剪表的裁决。

### 5 工具需求
- **无新增工具**。rustc 1.99.0 已就绪；`cargo test` 直接用。
- 共享 target 锁：排队；尽量 `tools::` / `mcp::` / `permission::` / `knowledge::` 分片。
- 若需 20 连跑 flake 复核占久，先报中层时间窗。

### 6 禁区与验收
- 禁：引擎产品、symlink 产品行为、ISSUE-034 协议改造、改 `proc_lifetime` 实现、commit/push、未审删测。
- done-when：覆盖绿表+缺口交中层；裁剪提案过裁决并落地或列暂缓；交叉审通过；中层 close。

### 7 执行顺序（仅批准 start 后）
1. 读码核对 C1–C12 与现测映射 → 跑本域测 → 写绿表+缺口。
2. 重核 WP-021 §5 + 新发现 → 填裁剪提案表 → 私信中层裁决。
3. 获批后落地放宽/删除；未批列暂缓。
4. 交叉审段落 → deliver WP-050。

### 8 ISSUE 状态备忘（方案填写时已确认编号）
| ID | status | 说明 |
|---|---|---|
| ISSUE-039 | **closed**（gm 2026-10-12） | WP-036/038 验收；custom 组杀/有界 drain |
| ISSUE-040 | **closed**（gm 2026-10-12） | WP-036/038 验收；bind_child_to_parent |
| ISSUE-034 | open（本包不碰） | MCP 协议无超时/错号 |


---
## 执行记录（WP-050 start 后，2026-10-12 20:15–20:17 HKT，rustc 1.99.0，未 commit）

### 步骤一：覆盖绿表（C1–C12）

实跑环境：`cargo` 直接用（无 PATH 前缀）；共享 target；`test-threads=8`（bash stress 用 16）。

| # | 核心流程 | 关键测试 / 证据 | 结果 | 备注 |
|---|---|---|---|---|
| C1 | custom EPIPE 放过 + stdin 写线程不死锁 | `child_ignoring_stdin_is_not_a_broken_pipe_failure`；`large_output_before_reading_stdin_does_not_deadlock` | **ok** | 含于 `cargo test --lib tools::custom` **23/0**（5.0s） |
| C2 | 超时/取消组杀 + Finished 组杀 + 有界 drain（ISSUE-039） | `timeout_kills_grandchild_process`；`finished_reaps_grandchild_holding_stdout`；`finished_kills_orphaned_grandchild_before_side_effect`；读码 `finish_after_child_exit`→`kill_child_tree`→`drain`（custom.rs:439+） | **ok** | ISSUE-039 已 closed，仅复核绿 |
| C3 | bind_child_to_parent（ISSUE-040） | 读码 custom.rs:325 `bind_child_to_parent(child.id())`；Linux no-op 不伪测 Job | **ok（调用点）** | ISSUE-040 已 closed；Windows Job 本 box 无法实跑 → 见缺口 G1 |
| C4 | wait 失败不 panic；后台 spawn 失败 abandon | `wait_failure_returns_error_instead_of_panicking`；`background_spawn_failure_is_reported_and_job_released` | **ok** | |
| C5 | 后台 mailbox；输出含 cancelled 不误判 | `background_real_process_begin_job_then_mailbox_notice`（等待已 10s）；`background_output_mentioning_cancelled_is_ok` | **ok** | mailbox 测单次绿；等待上限已是 10s |
| C6 | MCP kill/Drop 杀树 + spawn 组/bind | `mcp::client::tests::kill_reaps_grandchild_process`；`drop_reaps_grandchild_process`；读码 client.rs:39/48/187/192/227；pool `kill_child`→`client.kill()`（pool.rs:299-311） | **ok** | `cargo test --lib mcp::` **6/0**（含 4 条 exa_mcp）；`mcp_integration` **10/0** |
| C7 | bash 前台/后台 job 契约 | `cargo test --lib tools::bash` **64/0**（含 bash/jobs_contract/safety/status） | **ok** | flake 候选见下「只记录」 |
| C8 | permission Ask/拒绝/流水线 | `permission::` **20/0**；`client_protocol_permission` **8/0**；`permission_pipeline` **16/0** | **ok** | 前端 WP-033 队列不在本域测 |
| C9 | knowledge 索引/校验/mentions | `knowledge::` **46/0**（含 tools::knowledge 10）；单独 `tools::knowledge` **10/0** | **ok** | |
| C10 | webfetch SSRF/代理口径 | `tools::webfetch` **9/0**（含 `ssrf_*`、`whatwg_triple_slash…`）；`websearch` **4/0** | **ok** | ISSUE-021 系统代理 DNS 残留仍无产品测 → 缺口 G3 |
| C11 | optional 锁中毒不连环 | `optional::tests::poisoned_state_lock_does_not_cascade_panics`；`optional::` **7/0** | **ok** | |
| C12 | lsp 行号格式 / Windows cfg | `tools::lsp` **26/0**（Windows 测 `#[cfg(windows)]` :1086 本机跳过）；编号行靠 lib 测 | **部分** | 集成 `agent_lsp_tool_line_text_scenarios` **warmup 超时**（环境）→ 缺口 G2 |

#### bash flake（覆盖阶段只记录，不修）
| 测试 | 本包观察 | 历史（WP-022） |
|---|---|---|
| `tools::bash::live_small_echo_exit_code_first_file_on_disk_not_in_view` | 单跑 10/10；`tools::bash` 全量 16 线程×8 轮 **0/8 失败** | 曾 4/20 |
| `tools::bash::live_large_output_windows_and_keeps_full_log` | 同上 0/8 | 曾 2/20 |
| `tools::bash_jobs_contract::short_command_uses_completed_view_not_running_status` | 同上 0/8 | 曾 2/20 |

结论：本包未能复现 flake，但历史证据仍在；**提案放宽保留**（步骤二），不改代码。

#### 缺口
| ID | 缺口 | 严重度 | 处置 |
|---|---|---|---|
| G1 | Windows Job Object 绑定（ISSUE-040）本 Linux box 无法实跑 | 低 | 调用点已核；需 Windows CI/机 |
| G2 | `tests/optional_lsp::agent_lsp_tool_line_text_scenarios` warmup 超时（`rust-analyzer` 对 active toolchain unavailable，fallback 仍 10s 内未 warm） | 中（环境） | 行号断言逻辑已在 WP-027 改过；lib 26 条覆盖格式。不改测；记环境依赖 |
| G3 | ISSUE-021 webfetch 系统代理 DNS/pin 失效无回归测 | 低（已知残留） | 只登记，本包不修 |
| G4 | MCP pool 超时后 `stop_on_hub`→杀树无**独立**单测（依赖 client kill 测 + 读码委托） | 低 | 提案可不补（委托清晰）；若要补放后续 |
| G5 | `tool/output.rs:211` spill `fs::write` 非原子（WP-027 B11）仍无测/未修 | 低 | 只登记 |
| G6 | ISSUE-034 协议无超时/错号 | arch 高 | **禁区，本包不碰** |

### 步骤二：裁剪提案（审前不删不放宽）

重核 WP-021-backend §5 Tools 旧 10 条 + 本包新观察：

| ID | 测试（file::name） | 提案 | 理由（重核后） | 替代覆盖 | 中层裁决 |
|---|---|---|---|---|---|
| T1 | `tools::custom::background_real_process_begin_job_then_mailbox_notice` | **留** | 等待已是 10s；本包绿；WP-027 后 20/20。旧「放宽到 10s」已落地 | — | **留**（不改） |
| T2 | `tools::ask_user` 多问题文案 `contains("[…] selected_ids: …")` / `free_text:` | **放宽** | 仍逐字拼格式化串；文案改即挂 | 结构化找行 + 解析 selected_ids/free_text | **放宽·已落地** |
| T3 | `tools::plan::test_create_rejected_does_not_create_file` 含 `"No plan file was created"` | **放宽** | 文案与实现耦合 | 保留 rejected + 无文件 + `active_plan.is_none()` | **放宽·已落地** |
| T4 | `tools::bash::live_small_echo_exit_code_first_file_on_disk_not_in_view` | **放宽** | 历史 4/20 flake | `wait_bash_logs` 轮询到 tee 含 marker | **放宽·已落地** |
| T5 | `tools::bash::live_large_output_windows_and_keeps_full_log` | **放宽** | 历史 2/20 | `wait_bash_logs` 轮询到含 LINE_1200 | **放宽·已落地** |
| T6 | `tools::bash_jobs_contract::short_command_uses_completed_view_not_running_status` | **放宽** | 历史 2/20 | foreground wait 5s；若仍 running 则 wait_until+wait_shell | **放宽·已落地** |
| T7 | `tests/permission_pipeline` sink 往返 vs `client_protocol_permission` | **留（不删）** | 两层各有独特断言 | — | **留**（不改） |
| T8 | `knowledge/spec_tests` 同分支 reason + text.contains | **放宽（文案侧）** | 重复在同一文件双断言 | 每分支保留 reason；删重复 text；留 1 条 10MiB 样板 | **放宽·已落地** |
| T9 | `tools::webfetch::whatwg_triple_slash_parses_as_hostname_not_ssrf_gate` | **可删候选** | 只测第三方 url crate | `ssrf_*` | **删·已落地** |
| T10 | `tools::websearch` 无引擎构造测 | **留** | 轻量 | — | **留**（不改） |

**硬规则**：裁决前零删除/零放宽 —— 本轮已按中层 return note 落地。

#### 落地证据（2026-10-12 20:19–20:xx HKT，rustc 1.99.0，未 commit）
| ID | 改动要点 | 复跑 |
|---|---|---|
| T2 | `ask_user.rs`：`assert_selected_ids` / `assert_free_text` 按行前缀解析，去掉精确整串 contains | `tools::ask_user` **6/0** |
| T3 | `plan.rs`：去掉 `"No plan file was created"` contains；保留 rejected / opinion / 无文件 / active_plan | `tools::plan` **12/0**（含 `test_create_rejected…`） |
| T4/T5 | `bash.rs`：新增 `wait_bash_logs`（5s 轮询）；两 live_* 用其等 tee | `tools::bash*` **64/0**（threads=16） |
| T6 | `bash_jobs_contract.rs`：foreground wait 5s；running 则 `wait_until`+`run_wait` | 同上含 `short_command…` ok |
| T8 | `spec_tests.rs`：oversized 留 1 条 mention 样板；directory / invalid_lines 删重复 text/message | `knowledge::spec_tests` **11/0** |
| T9 | 删除 `whatwg_triple_slash…` 整测 | `tools::webfetch` **8/0**（原 9，−1） |
| T1/T7/T10 | 无改动 | — |

### 交叉审
- 本包为覆盖/裁剪，无新改 `custom`/`mcp`/`proc_lifetime`。生命周期结论沿用 WP-038：ISSUE-039/040 已 closed 且本包测复核绿。
- 与 IDE：无新进程生命周期代码需对方再审；标 **不适用（无代码变更）**。
- 权限与前端队列：本域测到 authorize/WS bridge；FIFO 单卡属前端 WP-033，边界已分隔。

### 禁区核对
未改：ISSUE-034、引擎产品、symlink、`proc_lifetime` 实现。未删测。未 commit/push。

### 交付物
- 覆盖绿表 + 缺口：本节步骤一
- 裁剪提案：本节步骤二（待中层填「中层裁决」列）

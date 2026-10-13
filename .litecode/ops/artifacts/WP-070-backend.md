# WP-071 / WP-070-backend：LSP/Tools 收尾小活（后端中层拆包）

- 父包：WP-070（GM，Andy 批，范围锁定）；中层包：WP-071（owner backend_mid，approver gm）
- 分支：`ops/team-framework`；**不 commit/push**（GM 统一合包提交）
- 本阶段：拆子包 + 方案提交 GM 审；**未 start**

## 1. 子包（可并行，无共享文件）

| WP | Owner | 范围 | Issue | 档位 | 产物 |
|---|---|---|---|---|---|
| WP-072 | tools | `src/tools/litecode_workspace.rs:821` 不再自开 sessions.db，经 session 层；裁定 `tests/optional_lsp.rs:1409` `agent_lsp_tool_views_with_rust_analyzer` 文案（`→ 1 unique` vs `→1 unique`）+ 等级断言：改测 or 改 formatter，并落地 | ISSUE-042；1409（非产品/测试过时，无 issue） | 架构越界（drift）/ 测试过时 | WP-070-tools.md |
| WP-073 | ide | `src/lsp/deps.rs:412`（rustup which）、`:446-452`（version probe）加 `.stdin(Stdio::null())`；`src/lsp` hub 对 -32801 ContentModified 有限重试（`rust_analyzer_smoke` :1488） | ISSUE-043（边角 bug）、ISSUE-044（常见 bug） | 边角 / 常见 | WP-070-ide.md |

文件面：WP-072 = src/tools/*、src/session*（只读/接口使用）、tests/optional_lsp.rs:1409 附近或 Tools formatter；WP-073 = src/lsp/*、tests/optional_lsp.rs:1488 附近（如需）。若两者都要动 tests/optional_lsp.rs，各改自己测函数，合并前中层核对不冲突。

## 2. 共同规则
- optional_lsp 测试**必须带 `</dev/null`**：`cargo test --test optional_lsp <name> -- --nocapture </dev/null`（真 RA 测加 `--include-ignored`）
- 以 cargo test 验证；ISSUE-042 复验 `cargo test --test session_data_sql_death_list`
- 不 commit；真机验证不在范围
- **`.github/workflows/windows-*.yml` 工作区改动不纳入本次统一提交**（Andy：保持原样，不碰）

## 3. 流程
1. GM 批 WP-071 → backend_mid start
2. 子包 fill+submit 方案 → backend_mid 审批 → 并行执行
3. 子包 deliver → 中层复跑验收 → close → WP-071 deliver 给 GM

## 4. 本阶段已办
- 新建 ISSUE-043、ISSUE-044（owner ide，wp WP-071）
- 建 WP-072、WP-073（drafting）
- validate_all 见提交记录

## 5. 追加（GM 批）：ISSUE-045 排队
- 新建 **ISSUE-045**（owner ide，产品缺陷）：LSP server 启动用 canonicalize 路径，rust-analyzer 链到 rustup 时 argv[0]=rustup 启动失败（deps.rs:208-211 → :304-319 → hub.rs:318/338/391；可用性经 is_rustup_shim 回退误通过）。修法：原始路径启动，canonical 仅用于 shim 识别；补 2 测。mise/volta shim 按机制推断，未实测。
- **排队为 IDE 第二子包，WP-073 done 后串行开**（同文件面 src/lsp/deps.rs、hub.rs，避免冲突）；本阶段未建 WP。

## 6. 交叉审 / 验收：WP-072（Tools）— done
- diff：`src/tools/litecode_workspace.rs` call_inner → `Some(self.sessions.reader())`；`tests/optional_lsp.rs:1409` 期望 `→ 1 unique`（与 formatter lsp.rs:455 / 单测 lsp.rs:841 一致）。无越域改动（src/lsp/* 为 WP-073 进行中改动）。
- 中层复跑全绿：session_data_sql_death_list 2/2；lib litecode_workspace 15/15；subagent_session_isolation 15/15；`agent_lsp_tool_views_with_rust_analyzer -- --include-ignored </dev/null` 1/1（真 rust-analyzer，等级断言通过；日志 /tmp/wp072_lsp.log）。
- ISSUE-042 closed（by WP-072）；未 commit。

## 7. 交叉审 / 验收：WP-073（IDE）— done
- diff 仅 src/lsp：conn.rs 新 `request_retry_modified`（只读方法白名单，-32801 退避 200/500/1000ms，受 request_timeout 预算约束，带 rpc_id 透传不重试）+ 单测；server.rs 切到重试层；deps.rs / install/{mod,npm}.rs / adapters/rust_analyzer.rs 探测/安装 Command 加 `Stdio::null()`。
- 中层复跑：optional_lsp `</dev/null` 24/24（1 ignored）；无 `</dev/null`（`sleep | cargo test`）24/24，无 warmup timeout；`rust_analyzer_smoke --ignored` 1/1；lib lsp::conn 15/15。日志 /tmp/wp073_{a,b,c}.log。
- lib lsp::tests 3 败（normalize_makes_verbatim_path_prefix_compatible / publish_diagnostics_uri_matches_drive_case / uri_to_path_recovers_mangled_verbatim_file_uri）：HEAD 99a01f9 临时 worktree 基线同样失败 → 既有，非回归；立 **ISSUE-046**（ide，边角/测试环境，Windows 专用测未 cfg-gate），本波不修。
- ISSUE-043/044 closed（by WP-073）；未 commit。
- 下一步：**WP-074**（IDE-2，ISSUE-045 原路径启动）已建 drafting，待 IDE fill/submit。

## 8. WP-074 退回 + 新立 ISSUE-047/048
- **WP-074 验收退回（review→in_progress）**：按 Andy 10-13 语言通用规则 + WP-070-ide-045.md 中 VS Code/Zed 对照，删除 deps.rs rustup 专用分支（is_rustup_shim、rust_analyzer_runnable、program=="rust-analyzer" 分支，约 :420-445、:517-522），统一走 run_version_probe（cwd=项目根）；更新 T1；新增测「组件缺失→非零退出→报告 unavailable」；保留原始路径启动。
- **ISSUE-047**（ide，产品未定）：-32801 策略。现 WP-073 重试为自设；VS Code 返回空/取消并声明 staleRequestSupport.retryOnContentModified，Zed 视为 rejected。A=照 VS Code 不重试+提示；B=保留重试+initialize 声明。IDE 推荐 B，待 Andy/GM 定。
- **ISSUE-048**（ide，架构）：src/lsp 残留单语言特例（deps.rs:525-532 pyright 跳过 --version、:536-556 硬编码项目标记、:631 安装硬编码 rustup；project_root.rs:52-53、:180-201）；toolchain override（rust-toolchain）属产品未定。本包不修。

## 9. WP-074 第二轮审（review，未 close，待真 symlink 跑）
- rustup 专用分支已删（is_rustup_shim / rust_analyzer_runnable / command_runnable 中 program=="rust-analyzer"）；run_version_probe(binary, cwd) 经 command_runnable_command_in，hub.activate 与 server_map 传项目根；原路径启动（absolute_unresolved）+ Stdio::null 保留。
- src/lsp 剩余 rustup/rust-analyzer 特例均属 ISSUE-048：deps.rs:53 install_hint、:510 sources_for_server Cargo.toml、:602 安装 rustup component add；project_root.rs:52、:180。
- 复跑：lib lsp:: 114 过 / 3 败（仅 ISSUE-046）；T1/T2/T3 3/3；optional_lsp </dev/null 24/24（1 ignored）。真 RA 测待工具集管家恢复 symlink 后跑。
- kb 第 90 条 warning = symbol_drift：lsp-hub-lifecycle.md:12 引用 `impl LspHub › fn activate` 与 HEAD 不同——本波 WP-074 改 hub.rs activate（+probe_cwd）所致；节点 pending、语义未变，属预期，合包 commit 后自然消失。

## 10. 变更：ISSUE-047 定 A，并入 WP-074（第3轮退回）
- Andy 10-13 决定 ISSUE-047 选 **A**。WP-074 由 review 退回 in_progress（第2轮实现本身验收通过，退回仅为扩范围）。
- 追加范围：删 WP-073 自设 -32801 重试层（conn.rs ~:508）；照 vscode-languageclient client.ts:2291-2331——semantic tokens / resolve 类→取消，其余→空结果；工具输出明示「服务器正在更新，结果可能不完整」；注明出处；更新 smoke/重试测试。保留原路径启动 + 通用探测；ISSUE-048 不碰。
- 影响：ISSUE-044（WP-073 已 closed）的重试实现被 A 取代；ISSUE-047 于 WP-074 验收时 close。
- WP-071 **不 deliver**，待 WP-074 第3轮验收。

## 11. 新子包 WP-075（Tools-2，ISSUE-047 A 工具侧）
- owner tools / approver backend_mid，drafting，待 Tools fill/submit。
- 范围：src/tools/lsp.rs :649、:661 改用 hub.tool_action_outcome（.text 同前）；content_modified→with_warning（:683），:661 循环任一为真即加；文案「服务器正在更新，结果可能不完整」（英文示例：language server is updating (ContentModified); result may be incomplete — retry in a few seconds, do not treat as 'not found'）；补 1 测。依赖 WP-074 第3轮 hub API（已在工作区）。产物追加 WP-070-tools.md。
- WP-074 留 review，待 WP-075 落地后联合复跑再 close（ISSUE-045/047 同时 close）。

## 12. WP-075 测试裁定 + ISSUE-049
- **WP-075 测试裁定：选 A**——测试侧对 ContentModified 警告做有限轮询（出现警告即等待重试，次数有上限），并收紧 hover 断言（断言真实内容而非宽松匹配）；产品侧不加重试（与 ISSUE-047=A 一致）。
- **ISSUE-049**（ide，边角 bug / 测试环境）：optional_lsp 经 clear_mock_ls_env / mock LS env 改进程级 env，整组并行时 rust_analyzer_smoke 串用他测 mock 配置而失败，单跑通过；WP-073 定责（/tmp/lsp_full.log）与 WP-075 运行均见。本包不修。

## 13. WP-074 + WP-075 联合验收（done）
- 审 diff：conn.rs 自设重试已删，`request_outcome` 照 vscode-languageclient handleFailedRequest（client.ts，semantic tokens/resolve→取消，其余→`LspReply::ContentModified` 空结果），不重试；`rpc_id` 透传保留原始 -32801（单测 content_modified_passthrough_rpc_id_keeps_raw_error）；server.rs initialize 声明 `staleRequestSupport.retryOnContentModified: []`；hub `LspToolOutput`/`tool_action_outcome`；tools/lsp.rs :649/:661 用 outcome，content_modified→`CONTENT_MODIFIED_WARNING`；optional_lsp 测试侧 20s 有限轮询 + hover 断言收紧为含 `target`。

| 跑法 | 结果 |
|---|---|
| lib lsp:: | 116 过 / 3 败（仅 ISSUE-046） |
| lib tools::lsp | 27/27 |
| optional_lsp `</dev/null` ×3 | 24/24 ×3（1 ignored） |
| optional_lsp `--include-ignored` | 24 过 / 1 败：rust_analyzer_smoke 读到 mock server 配置（ISSUE-049 env 串用） |
| 真 RA 单跑 views / smoke | 1/1、1/1 |
| HEAD 基线（临时 worktree）×3 | 23/1 ×3（views 为旧期望/旧 canonical 启动失败，属本波已修） |

- edit_fix_must_not_echo_previous_lsp_error（:836）flake：当前 4 次 + 基线 3 次 0/7 复现；mock 不发 -32801，与本波无关 → 并入 ISSUE-049。日志 /tmp/wp074j/。

## 14. 总结（交 GM）
- 子包：WP-072、WP-073、WP-074、WP-075 全部 done。
- 已关 issue：ISSUE-042、043、044、045、047（044 的重试实现已被 047=A 取代）。
- 仍开 issue（本包不修）：ISSUE-046（Windows 路径测未 cfg-gate）、ISSUE-048（src/lsp 单语言特例 + toolchain override 产品未定）、ISSUE-049（optional_lsp 进程级 env 串用/并行 flake）。
- validate_all ok:true；kb 90 warnings，其中新增 1 条 symbol_drift（lsp-hub-lifecycle → hub.rs activate）为本波预期，合包 commit 后消失。
- `.github/workflows/windows-*.yml` 工作区改动**不纳入**统一提交（Andy：保持原样）。
- 未 commit / push；等 GM 统一合包提交。

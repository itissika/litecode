# WP-208 dig-survey（只读，基线 1ec99ae ≈ 3acfe31）

## 0. 现状计数
- lib：`cargo test --lib -- --list` = **2212**（含 3 ignored）。
- tests/：59 文件，WP-207 删后实跑 **502**（源码 `#[test]` 440；差额来自 `tests/common/permission.rs` 的 2 条测被 `mod common` 编进约 30 个 binary 重复跑）。
- 合计 **2714**。目标 **≤1000**（lib ≤800，tests/ ≤200）。

## 1. 按区：删前 → 目标 → 守的核心链路（代表测试）
| 区 | 删前 | 目标 | 核心链路（保留的代表测试） |
|---|---|---|---|
| tools（read/grep/glob/write/edit/apply_edits/run/wait/kill/lsp/webfetch/websearch/code/subagent/…） | 398 | 150 | 每工具：成功形状、not_found/invalid_input、越界/敏感路径、取消、并发写一致；write expected_hash stale；edit 扁平契约；run 前台 30s/后台 null、锁只到 spawn（tools::run_lock_gate 7 全留）；code：uncaught/timeout/cancel 封口（tools::code::*）。代表：tools::read::tests::cancelled_read_stops_between_chunks、tools::write::expected_hash_tests::write_expected_hash_mismatch_is_stale、tools::run_lock_gate::four_parallel_write_runs_finish_in_about_one_second |
| engines（session_search 188 / code_search 116 / tests 34 / 其它） | 369 | 110 | session_search：查询→排序→溢出文件可读→hit 与 read 一致（WP-205 回归）、索引增量；code_search：索引 reconcile、lexical 命中、index_status。golden/parity 只留各 1–2 条 |
| session（data::sqlite 85、manager 45、snapshot 25、transcript_file 15…） | 228 | 90 | 数据不丢：crash turn 封口、append seq 连续、compaction checkpoint、final_immutability、并发写；manager turn 生命周期；transcript 虚拟路径 |
| llm（codec 164：responses 37/messages 33/chat 22/stream_contract 16/http 12/sse 7…） | 192 | 70 | 每 codec 1 组核心流式（文本/工具调用/结束/错误）+ stream_contract 全留 + sse 分帧硬错误；wire_dump/replay 各 1 |
| workspace（filter 61、watcher 25、git 13、tree 12…） | 163 | 45 | 路径解析与越界、ignore/glob basename 匹配、watcher 事件去抖 1–2 条 |
| config（global_db 30、settings_writer 27、workspace 19…） | 126 | 40 | preset 展开、global db 迁移/seed、settings 原子写 |
| tool（executor 21、schema_validate 20、registry 17、availability 12、resource_gate 8、sdk_snapshot 2…） | 122 | 60 | **sdk_snapshot 全留**、resource_gate/write_lock 全留、executor 审批/取消/超时、schema 未知参数 |
| lsp（install 29、tests 25、conn 16、deps 13…） | 114 | 35 | 冷启动 unavailable（WP-206 #3）、导航重试、ContentModified、相对路径（#8）、didChange 同步 |
| runtime（pending_auto_turn 14、llm_resolve 13、stream_projection 12…） | 78 | 30 | turn 循环、stream projection 生命周期写、llm 解析 |
| client_protocol | 66 | 20 | connection 握手/鉴权、controller 权限往返、ws 隔离 |
| context_pipeline（keep_recent 15、system 11、budget…） | 61 | 25 | 预算估算、阈值（manual 30%）、keep_recent、provider bill |
| ptc（tests 49 + boundary…） | 58 | 40 | 序列化/undefined/bigint 路径、预算/取消/超时、栈与内存封口（WP-207 回归）、render 预算 |
| terminal | 34 | 15 | agent_jobs 前台窗/detach/wait/kill、pty |
| permission | 24 | 20 | floor、preset、subagent ask→deny（安全底线，基本全留） |
| 其他（knowledge 38、provider_catalog 35、reminder 32、types 21、optional 14、telemetry 11、serve 10、mcp 5、cli 4、misc 9） | 179 | 40 | provider catalog 唯一来源、mcp 冲突、serve 绑定鉴权；types death_list 并入单文件 |
| **lib 小计** | **2212** | **≤790** | |
| tests/（59 文件） | 502 | ≤200 | 见 §3 |
| **合计** | **2714** | **≤990** | |

## 2. 删除规则
**整类删：**
1. 退役方言"不存在"测（名含 legacy/retired/removed/old/dialect/no_longer/stale_：lib 约 80 条命中）——无回潮路径即删。
2. 描述/提示词子串断言（名含 description/prompt/mentions/wording 约 81 条命中 + 测体内 contains）——SDK 快照为唯一文本守卫；registry/subagent/knowledge 描述删断言（见 Q2）。
3. 琐碎测：serde 往返、Default、Display/Debug、getter、常量自证（名命中约 79 条）。
4. 参数化近似变体：同行为 >2 条近似测只留 1 条（不改写为表驱动，见 Q3）。
5. 私有 helper 测：被公共路径覆盖即删（engines/session_search 的 slots/sparse/chunk/query_plan、code_search enclosing、workspace filter 内部函数等为主）。
6. death_list：types::death_list_gate 13 + tests/*_death_list 18 → 保留确有现役回潮路径的 ≤6 条，合并到 `tests/architecture_guards.rs`；其余删。
7. tests/ 与 lib 单测重复的集成测删；`tests/common` 内的 #[test] 移除（每 binary 重复跑）。
8. golden/parity/probe（wp028_probe_*、*_golden）：只留 1 条代表。

**必须保留：** 安全底线（permission floor、越界/敏感路径、serve 鉴权）、并发与锁（resource_gate、write_lock、run_lock_gate、code_surface 并发写/审批锁）、数据不丢（session sqlite crash/immutability/compaction、write 原子与 expected_hash）、契约快照（sdk_snapshot、tools::baseline fixtures）、引用了 commit/WP/ISSUE 的真实回归（如 issue_032_changed_files_restore、wp098_follow_up_queue、WP-205 spill/hit、WP-207 stack seal、WP-206 run 锁）。

## 3. tests/ 逐文件初判（502 → ≤200）
- 留（核心/安全/并发，可内部再删重复）：code_surface 34→20、permission_pipeline 16→12、code_tool_runtime 4、run_code_model_view 17→6、session_data_{concurrency,fault,consistency,crud,lifecycle,bootstrap} 19→10、agent_loop 11→5、agent_e2e_responses 4→2、serve_bind_auth 1、g3_auth 4→2、f2_lock_scope 2、wp028_probe_s2_create_race 2→1、issue_032 4→2、wp098 5→2、terminal_ws_isolation 1、terminal_pty_contract 1、workspace_process_contract 5→2、subagent_child_turn_contract 8→3、subagent_session_isolation 15→5、manual_compact_e2e 2、compact_failure_calibers 5→2、mcp_integration 10→4、optional_lsp 25→8、optional_code_search 19→6、code_search_ipc 11→4、client_protocol_permission 7→3、wire_seq_identity 13→4、turn_prefix_stability 13→4。
- 大幅删：settings_api 48→12（UI 设置 CRUD 细节）、client_protocol_project 26→6、stage_a_ctx_consistency 27→6、workspace_api 13→4、session_list_empty_filter 13→2、subagent_send/tool_pipeline 15→5。
- 整文件删：architecture_layers（并入 architecture_guards）、*_death_list ×5（并入）、wp028_probe_s1_xdev_move、working_set_identity、workspace_path_{lap,consumer}（与 lib workspace 重复）、optional_p0、global_db_seed（与 config::global_db 重复）、session_turn_entry、session_transcript、session_delete、pending_messages、subscribe_bash_hydration、client_protocol_telemetry。
- 合计约 190。

## 4. 风险
- 大删后 dead helper（`tests/common`、`#[cfg(test)]` helper fn）需一并删，否则 dead_code 告警；按组编译清理。
- 只看名字分类会误删回归测：删前先 grep 测体/注释中的 WP-/ISSUE-/commit 引用，命中者转人工判定。
- 行覆盖：若装得上 `cargo llvm-cov`（box 可装），对 ptc/tools/session/llm 做删前删后抽查（可选）。

## 5. Q（带推荐）
- Q1 目标是硬上限还是指导线？推荐按 ≤1000 硬执行；若某区为守核心链路需超标，写理由，总量仍 ≤1000。
- Q2 registry/subagent/knowledge 描述断言：推荐直接删（不扩快照），这些面不在 run_code 提示面；备选扩 SDK 快照。
- Q3 参数化变体：推荐只删不改写（留 1 条代表），不新建表驱动测试。
- Q4 tools::baseline fixtures 与 code_surface 并发/审批测是否视为"契约快照/并发"必保？推荐是。
- Q5 死 helper 及 `tests/common/permission.rs` 的 2 条测：推荐删（每 binary 重复跑约 60 次）。
- Q6 是否装 `cargo llvm-cov` 做行覆盖抽查？推荐做 ptc/tools 两模块删前删后抽查（约 +10 min 编译），不作硬门槛。

# WP-047 方案（Session：核心覆盖核对 + 测试裁剪提案）

- owner: session　approver: backend_mid　parent: WP-043　child: WP-047
- 产物：`.litecode/ops/artifacts/WP-043-session.md`
- 状态：裁剪 T1–T4 已落地并复跑绿 → deliver（覆盖已过；T5–T8/N1/N2 按裁决不落地）
- 禁区：引擎产品、symlink 产品行为、ISSUE-032/034；不 commit
- 工具链：`rustc 1.99.0`；`cargo test` 在 `/workspace/litecode` 直接跑（**无 PATH 前缀**）；共享 target 遇锁排队；本域过滤优先
- 工具需求：**无**（现有 wp.mjs / cargo / rg 足够）

## 0. 硬串行与边界

两步**硬串行**，覆盖未完成前不做裁剪裁决落地：

1. **覆盖**：核对 Session 核心流程有效覆盖 → 绿表 + 缺口清单  
2. **裁剪**：列冗余/过时/重复及删/留/放宽提案 → **交 backend_mid 裁决；未过审前不删**

本域范围：`src/{session,client_protocol,serve}` + 对应 `tests/session_*`、`tests/client_protocol_*`、`tests/serve_*`（若有）、以及与 Session 协议入口交叉的 compact 相关集成测（`manual_compact_e2e` / `compact_failure_calibers` / `client_protocol_project` 中本域断言）。不改 Runtime/Context 实现；跨域只读引用。

参考（须**重核现状**，不盲抄）：
- WP-021-backend §5 Session 旧裁剪清单（8 条）
- WP-022-session（T1–T8 + G3–G5 落地情况）
- WP-033 口径 #3：Session DB 为唯一真相；reload 失败不得回滚压缩前内存
- WP-036 / WP-039：compact DB 真相、`compaction_failed`/`Succeeded`+`Ok(true)`、expected_revision fail-closed；Session 交叉审已有条件接受

---

## 1. 步骤 A — 覆盖核对

### 1.1 核心流程清单（映射表骨架）

开工后产出「流程 → 现有测试(file:line) → 实测结果 → 缺口」表。拟覆盖的核心流：

| # | 流程 | 关注点 | 初步已知锚点（批后重核行号） |
|---|---|---|---|
| F1 | 开/关/恢复会话 | open / delete / resume；revision CAS；冷启动后 list | `session_data_{bootstrap,crud,lifecycle,fault}`；`session_delete`；manager `open_*` / `resume_session`（含 WP-024 S1 修后路径） |
| F2 | 回合生命周期 | Idle→StartingTurn→RunningTurn→Idle；TurnGuard；排队归还；finish_turn | manager 预约/释放；`session_turn_entry`；WP-024 G3/G5 manager 测 |
| F3 | compact 错误码（协议） | `-32000` + `error.data`：`agent_already_running`(retryable=true) / `compaction_failed`(false)；nothing_to_compact / cancelled | connection `compact_error_response`；`client_protocol_project`（含 `turn_finished_auto_compact_failure_is_compaction_failed`）；connection `compact_error_tests` / G4 |
| F4 | compact × DB 真相交叉（WP-033/036） | 落库成功后 reload 失败 → 不回滚、Succeeded+Ok(true)；fail-closed revision；Session 持久化侧零 diff 确认 | 只读对照 `context_pipeline/compact.rs` 回归 + Session `SessionMutation::Compact` / writer CAS；本域断言：协议侧生命周期通知与错误码不与「DB 已提交却 Failed」矛盾 |
| F5 | 重连后停回合、可再点 | 重连用尽 → TurnEndReason::Error → Idle；可再 `start_turn` / 再 compact | calibers `reconnect_then_*`（Runtime 主跑，Session 核协议/投影）；manager `finish_turn_clears_running…`；G5 缺口（JSON-RPC 全链路）是否仍开着 |
| F6 | Ask / 权限 / 连接（附带） | Ask registry 不互阻；断连不取消回合（设计）；permission wire | `ask_does_not_block…`；`client_protocol_permission`；ws 断连行为只读确认 |

### 1.2 方法

1. **映射**：对 F1–F6 用 `rg` 定位测试名与源码 file:line，写入绿表「覆盖列」。  
2. **实测**（仅读，不改代码）：  
   - `rustc -V` 确认 1.99.0  
   - lib：`cargo test --lib session:: client_protocol:: serve::`（或等价过滤）  
   - 集成：本域相关 `--test` 文件（session_*、client_protocol_*、serve_*、compact_failure_calibers、manual_compact_e2e）；共享 target 锁则排队；`--no-fail-fast`  
   - 已知假红：`session_data_sql_death_list`（session_search，ISSUE-041）只登记，不计入本域失败  
3. **绿表**：每行 = 流程 × 测试 × pass/fail × 日志路径（`.tmp/wp047-*.log`）  
4. **缺口**：无测试 / 仅 manager 层无 wire / 与 WP-033/036 口径冲突 / G5 协议全链路仍缺 → 列入「缺口」交中层，**本包不擅自补测**（补测另立或经批）

### 1.3 与 WP-033/036 的交叉核对（只读）

- 确认本域协议投影：auto compact 失败 → `compaction_failed` + `retryable=false`（WP-024/025 已落）仍绿  
- 确认 Session 侧未引入「reload 失败则回滚内存」的路径（WP-039 零 session diff 前提下，抽查 Compact mutation / lifecycle 通知）  
- pending-cache 同回合重复（Session 交叉审非阻断条件）：覆盖表记「已知软缺口」，不扩 scope

---

## 2. 步骤 B — 裁剪提案（覆盖完成后）

### 2.1 输入与重核

- 以 **WP-021-backend §5 Session（8）** + **WP-022-session T1–T8** 为起点  
- 对每条：`rg`/读测试确认仍存在、行号是否漂移、是否已被更强测试替代、WP-024 后是否新增重复  
- 标准：  
  - **可删**：被更强测试完全覆盖，或测已删除行为  
  - **可放宽**：过细文案/sleep 时序/源码 grep 守门易抖  
  - **重复**：同一路径多处断言 → 表驱动或留一条  
  - **登记**：假红/跨域（如 sql death_list）只登记不删  

### 2.2 输出格式（写入本产物 §裁剪）

| # | 测试路径::名称 | 类别 | 理由（对照现状 file:line） | 建议（删/留/放宽） | 替代覆盖 |
|---|---|---|---|---|---|

旧清单预期重核项（示意，批后逐条核实）：T1 telemetry 双方法常量；T2 permission 三条快照（已决议留一条完整 wire）；T3 list_empty_filter 六条表驱动；T4 lifecycle sleep(50ms)；T5 manual_compact_e2e vs calibers lease（Runtime 主张留 calibers 快回归）；T6 turn_entry 源码 grep；T7 deferred 死代码（实现侧可删）；T8 sql death_list 假红登记。

**未过 backend_mid 审前：零删除、零 `#[ignore]`、零放宽落地。**

---

## 3. 产物结构（批后填写执行节）

方案通过并 start 后，本文件追加：

1. 覆盖绿表（F1–F6）  
2. 缺口清单  
3. 裁剪提案表（重核后）  
4. 测试命令与日志路径  
5. 与 Runtime/Context 只读交叉备注（若有）  
6. 工具需求变更（若有）

---

## 4. 交付与门禁

- done-when（拟）：覆盖绿表+缺口已写；裁剪提案已交中层；本域实测记录完整；无擅自删测/改产品代码；禁区未触  
- deliver 交 backend_mid 验收；不 commit；不直接找 GM  
- 停手只停 Session 名下活；卡点先报 backend_mid  

## 5. 工具需求

**无。** 若后续需独占 target 或临时提高并行度，再报中层转工具集管家。

---

# WP-047 执行记录（Session）

- owner: session　approver: backend_mid　parent: WP-043　child: WP-047
- start: 2026-10-12 ~20:14 HKT（`wp.mjs start --as session --id WP-047` ok）
- 工具链：`rustc 1.99.0 (b940084d7 2026-09-28)`；`cargo test` 无 PATH 前缀；共享 target 无锁等待
- 禁区遵守：未改产品代码、未删/放宽/ignore 测试、未补缺口测、未 commit、未触引擎产品/symlink/ISSUE-032/034

## A. 覆盖绿表（F1–F6）

| # | 流程 | 关键覆盖测试（file:line） | 实测 | 日志 |
|---|---|---|---|---|
| F1 | 开/关/恢复会话 | `tests/session_data_bootstrap.rs:7` empty_workspace；`:23` reopen_with_held_lease；`session_data_crud.rs:21` create_append_meta_delete；`:51` parent_delete_cascades；`:133` conflict_on_stale_revision；`session_data_fault.rs` 三故障路径；`session_delete.rs:55` delete_listed_idle…cold_start；`manager.rs:3275` s1_reopen_prune_keeps_revision_in_sync；`manager.rs:601` resume_session 路径（S1 修后由 s1 间接覆盖） | **绿**（集成 bootstrap/crud/fault/delete 全过） | `.tmp/wp047-integration.log` + `.tmp/wp047-integration-rest.log`；lib `.tmp/wp047-lib-session.log` |
| F2 | 回合生命周期 | `manager.rs:3185` g3_turn_finished_reason_non_empty…；`:3231` g4_g5_compact_rejected…and_turn_restarts；`:3309` finish_turn_clears_running…；`tests/session_turn_entry.rs:68` bash_idle…releases_on_failure；预约/释放/TurnGuard 含 fanout 回滚测 | **绿**（lib session:: 含 manager 测；turn_entry 4/0） | 同上 |
| F3 | compact 错误码（协议） | `connection.rs:59` compact_error_response；`:1946` g4…agent_already_running retryable；`:1955` compact_real_failure…compaction_failed；`project.rs:1211` auto_compact_failure_maps…；`tests/client_protocol_project.rs:596` turn_finished_auto_compact_failure_is_compaction_failed；`:34/:55` compact_lifecycle wire | **绿**（client_protocol lib 29/0；project 26/0） | `.tmp/wp047-lib-client_protocol.log`；integration |
| F4 | compact × DB 真相（WP-033/036） | Context 主责：`compact_failure_calibers` / context compact；Session 侧：`SessionMutation::Compact` writer CAS（`data/writer.rs:810`）；协议不把「DB 已提交」投影成 Failed（F3 绿）；WP-039 交叉审已确认 **零 session diff**；本域抽查无「reload 失败回滚压缩前内存」路径 | **绿（交叉只读）**；calibers 7/0、manual_compact_e2e 4/0 | integration log |
| F5 | 重连停回合、可再点 | Runtime：`compact_failure_calibers.rs:297` reconnect_then_auto_compact_failure_stops_turn；`:342` reconnect_then_terminal_llm_error_stops_turn；Session：`manager.rs:3231` g4_g5…turn_restarts_after_error；`project.rs` turn_finished_marks_a_failed_reconnect_retryable（`:544`） | **绿（分层）**；**缺口 G5-wire** 见 §B | integration |
| F6 | Ask / 权限 / 连接 | `connection.rs:2178` ask_does_not_block_other_session_rpc；pending_ask_tests / ask_loop_tests；`tests/client_protocol_permission.rs` 8 条（含 bridge round_trip）；`tests/serve_bind_auth.rs:17` + lib serve:: 10/0 | **绿** | lib client_protocol / serve；integration |

### A.1 本域实测汇总

| 层 | 过滤 | pass | fail | 备注 |
|---|---|---:|---:|---|
| lib | `session::` | 207 | 0 | `.tmp/wp047-lib-session.log` |
| lib | `client_protocol::` | 29 | 0 | `.tmp/wp047-lib-client_protocol.log` |
| lib | `serve::` | 10 | 0 | `.tmp/wp047-lib-serve.log` |
| **lib 小计** | | **246** | **0** | 与 WP-024 基线一致 |
| 集成 | session_* / client_protocol_* / serve_bind_auth / compact_failure_calibers / manual_compact_e2e | 107 | 1 | 唯一失败见下 |
| **假红登记** | `session_data_sql_death_list::sessions_db_sql_and_rusqlite_stay_in_sqlite_module` | — | 1 | **归 ISSUE-041**；命中含 `src/engines/session_search/*` 及现亦扫到 `src/session/data/final_immutability_tests.rs`（门禁白名单未跟进，仍非本域产品回归） |

本域健康口径：**353 pass / 0 真失败**（假红外置 ISSUE-041）。

## B. 缺口清单（只列，本包不补测）

| # | 缺口 | 现状 | 建议归属 |
|---|---|---|---|
| G-wire / G5 | auto/错误终止后经 **JSON-RPC 协议入口** 再 `start_turn` 的全链路 e2e | 仅 manager 层 `g4_g5_…turn_restarts_after_error`（manager.rs:3231）；WP-024 已记「并入 Runtime G2 e2e」 | Runtime 后续 / 另立 WP |
| G-S7 | turn/end 落盘失败仅 `tracing::warn`，`TurnFinished.reason` 可能为 None | 正常路径 G3 已绿；失败注入路径仍缺 | Session 或 Runtime（suspect，非阻断） |
| G-pending | compact 落库成功 + reload 失败后同回合 pending-cache 可能重复；`end_turn` 后自愈 | WP-039 Session 交叉审非阻断条件 | Context/Session 软缺口，不扩本包 |
| G-death | sql death_list 假红持续；命中面扩大到 `final_immutability_tests.rs` | 登记 ISSUE-041；本包不删不改门禁 | ISSUE-041 owner |

无「与 WP-033/036 口径冲突」的本域红项；F3 `compaction_failed`/`retryable=false` 仍绿。

## C. 裁剪提案（重核 WP-021 §5 Session + WP-022 T1–T8；**只提案，零删除**）

对照 2026-10-12 工作树重核：旧 8 条均仍存在；行号有漂移处已更新。

| # | 测试路径::名称 | 类别 | 理由（现状 file:line） | 建议 | 替代覆盖 |
|---|---|---|---|---|---|
| T1 | `tests/client_protocol_telemetry.rs::subscribe_logs_is_jsonrpc_method`（:39）+ `::unsubscribe_logs_is_jsonrpc_method`（:49） | 重复 | 两条仅断言方法名常量字符串 | **放宽/合并**为一条或表驱动 | ws 订阅路径协议测（若有）+ 留一条常量守门 |
| T2 | `tests/client_protocol_permission.rs::server_hello_notification_serializes_correctly`（:8）、`::permission_request_serializes_request_id`（:94）、`::ask_kind_wire_snake_case`（:386） | 重复/可放宽 | 三条序列化快照重叠 | **留** `permission_request_serializes_request_id` 完整 wire；其余两条收字段级或删 | `ws_bridge_*_round_trip` + 保留快照 |
| T3 | `tests/session_list_empty_filter.rs`：`seq_hole_…`(:166) / `unknown_kind_…`(:185) / `surface_op_not_json_…`(:205) / `seq_starting_at_one_…`(:224) / `empty_surface_op_…`(:245) / `legacy_detail_kind_…`(:292) | 重复 | 同一「有 item 行就列出」规则 × 六种畸形 | **合并**表驱动一条（可保留 1–2 极端样例） | 表驱动 + `empty_new_session_stays_hidden` 等行为测 |
| T4 | `tests/session_data_lifecycle.rs` shutdown 测内 `sleep(50ms)`（:77） | 可放宽 | 靠 sleep 等时序，易抖 | **放宽**：改 notify/轮询 deadline | 同文件行为断言保留 |
| T5 | `manual_compact_e2e.rs::manual_compact_failure_e2e_releases_lease_and_retry_starts`（:65） vs `compact_failure_calibers.rs::manual_compact_failure_releases_lease_for_retry`（:203） | 重复（部分） | lease 释放再试路径重叠；e2e 更强；Runtime 主张 calibers 留作快回归 | **留两者**：e2e 主路径；calibers 降为引擎层断言（与 WP-022/Runtime 交叉审一致） | e2e + `manual_compact_retry_after_failure_completes` |
| T6 | `tests/session_turn_entry.rs::session_layer_has_no_subagent_orchestrator`（:39）、`::session_module_does_not_declare_jobs_or_turn_entry`（:92） | 可放宽 | 源码 grep 守门，改名/挪文件假红 | **放宽**：改模块可见性/编译期约束，或降为 lint 脚本 | 行为测 `bash_idle_…` / fanout |
| T7 | `src/client_protocol/connection.rs` deferred 消费（约 :1770–1848，`proj.deferred.drain` / `_dummy_deferred`） | 可删（实现侧） | 无生产者的遗留队列；属实现清理非测试 | **实现侧可删**（另包；本包不改代码） | 无需替代测 |
| T8 | `tests/session_data_sql_death_list.rs::sessions_db_sql_and_rusqlite_stay_in_sqlite_module` | 登记 | 假红；session_search + 现 `final_immutability_tests.rs` | **登记 ISSUE-041**；不删不 ignore | 门禁白名单由 ISSUE-041 修 |

**本包新增观察（非旧清单，供中层知悉，不擅删）：**

| # | 项 | 说明 |
|---|---|---|
| N1 | WP-024 新增 G3/G4 wire 与 manager 测 | 与 calibers/e2e 互补而非纯重复；**建议留** |
| N2 | `client_protocol_project` 投影测偏多 | 单字段投影测可远期表驱动；**本包不提案强删**，避免伤 wire 契约 |

**未过 backend_mid 审前：零删除、零 `#[ignore]`、零放宽落地。**

## D. 测试命令

```bash
cd /workspace/litecode
rustc -V   # 1.99.0
cargo test --lib session:: -- --test-threads=4
cargo test --lib client_protocol:: -- --test-threads=4
cargo test --lib serve:: -- --test-threads=4
cargo test --test session_data_bootstrap --test session_data_concurrency \
  --test session_data_consistency --test session_data_crud --test session_data_fault \
  --test session_data_lifecycle --test session_data_sql_death_list --test session_delete \
  --test session_list_empty_filter --test session_transcript --test session_turn_entry \
  --test client_protocol_permission --test client_protocol_project --test client_protocol_telemetry \
  --test serve_bind_auth --test compact_failure_calibers --test manual_compact_e2e \
  -- --test-threads=4
# 因默认 fail-fast，death_list 失败后补跑：
cargo test --test session_delete --test session_list_empty_filter \
  --test session_transcript --test session_turn_entry -- --test-threads=4
```

日志：`.tmp/wp047-lib-session.log`、`.tmp/wp047-lib-client_protocol.log`、`.tmp/wp047-lib-serve.log`、`.tmp/wp047-integration.log`、`.tmp/wp047-integration-rest.log`。

## E. 只读交叉备注

- Runtime R1/R4：calibers reconnect_* 与 Session F5 manager/wire 分层覆盖仍成立；G5-wire 仍开。
- Context WP-033/036/039：Session 侧无 reload-rollback；F3/F4 协议与「DB 权威」不矛盾；pending-cache 软缺口见 G-pending。
- 工具需求：**无变更**。

## F. 交付门禁自检

- [x] 覆盖绿表 + 缺口已写  
- [x] 裁剪提案已交中层（8+观察，零落地删改）  
- [x] 本域实测记录完整  
- [x] 无擅自删测/改产品代码  
- [x] 禁区未触

## G. 裁决 + 落地证据（backend_mid 2026-10-12）

### G.1 裁决采纳

| 项 | 裁决 | 本包动作 |
|---|---|---|
| 覆盖 F1–F6 | 抽查成立；353 pass / 0 真失败采纳 | 保留绿表；假红一律 **ISSUE-041**（不再写 WP-011） |
| 缺口 G5-wire / G-S7 / G-pending | 只列 OK | 未补测 |
| **T1** | 批：telemetry 两方法合并为一条/表驱动 | **已落地** |
| **T2** | 批：留 `permission_request_serializes_request_id` 完整 wire；删 `server_hello`/`ask_kind` 快照；无替代字段收单字段 assert | **已落地** |
| **T3** | 批：六条 empty_filter 并表驱动（表内 ≤2 极端样例） | **已落地** |
| **T4** | 批：`sleep(50ms)` → notify/轮询 deadline | **已落地** |
| T5 | 批留：e2e + calibers 都留 | 未改 |
| T6 | 本包留：源码 grep 暂不动 | 未改 |
| T7 | 本包不落地：deferred 死代码另包 | 未改 |
| T8 | 登记跟 ISSUE-041，不删不 ignore | 产物改挂 ISSUE-041 |
| N1/N2 | 留 | 未改 |

### G.2 落地 diff（仅测试；不 commit）

| 文件 | 变更摘要 |
|---|---|
| `tests/client_protocol_telemetry.rs` | `subscribe_logs_*` + `unsubscribe_logs_*` → `log_subscribe_methods_are_jsonrpc` 表驱动 |
| `tests/client_protocol_permission.rs` | 删 `ask_kind_wire_snake_case`；`server_hello_notification_serializes_correctly` → 单字段 `server_hello_omits_llm_ecosystem`；`permission_request_serializes_request_id` 扩为完整 wire（含 skip 空 options/questions） |
| `tests/session_list_empty_filter.rs` | 六条畸形行测 → `malformed_transcript_rows_still_listed_after_reopen`；表内极端样例：seq hole、unknown kind；保留 `last_message_with_only_control_plane_rows_still_listed` |
| `tests/session_data_lifecycle.rs` | `sleep(50ms)` → oneshot notify + `timeout(500ms)` + `yield_now` 后再 `abort` |

禁区：未改引擎产品、未 symlink 复制、未碰 ISSUE-032/034、未 commit。

### G.3 复跑证据

```bash
cd /workspace/litecode
rustc -V   # 1.99.0 (b940084d7 2026-09-28)
cargo test --test client_protocol_telemetry --test client_protocol_permission \
  --test session_list_empty_filter --test session_data_lifecycle -- --test-threads=4
```

| 套件 | 结果 | 日志 |
|---|---|---|
| `client_protocol_telemetry` | **3 passed** | `.tmp/wp047-trim-landing.log` |
| `client_protocol_permission` | **7 passed** | 同上 |
| `session_data_lifecycle` | **2 passed** | 同上 |
| `session_list_empty_filter` | **13 passed**（含 common 2） | 同上 |

合计相关：**25 passed / 0 failed**（2026-10-12 20:21 HKT）。

### G.4 交付门禁自检（落地后）

- [x] T1–T4 按裁决落地  
- [x] T5–T8 / N1/N2 按裁决不动  
- [x] 假红口径改为 ISSUE-041  
- [x] 相关测复跑全绿  
- [x] 无产品代码 / 无 commit / 禁区未触  

# WP-017 · Session 域 · 必过清单 + 实跑证据

- 域：session / client_protocol / serve
- 执行：Session bot · 2026-10-12 HKT
- 基线：WP-012 Session 节
- 工作路径：`/workspace/litecode`
- 工具链：`rustc/cargo 1.99.0`（`RUSTUP_HOME=/home/box/.rustup`；系统 `/usr/bin/rustc` 为 1.85，不可编）
- `HOME=/workspace/litecode/.tmp/home`（stdin `</dev/null`）
- **未** commit / push；**未**改产品形状
- 日志目录：`.litecode/ops/artifacts/wp017-session/`

---

## A) 必过清单

| 链路 | 过滤命令 | 测名要点 |
|---|---|---|
| **回合** | `cargo test --lib -- session::manager::child_session_tests` | begin/finish/reservation/fanout/remove/revert 等 22 |
| **回合** | `cargo test --lib -- config::turn_guard` | `begin_end_cycle` / `nested_turns` |
| **回合** | `cargo test --test session_turn_entry` | session 层无 orchestrator/jobs；fanout/bash idle 口径 |
| **回合** | `cargo test --test pending_messages` | 排队消息 seam 注入 / final 延长 / commit 公告 |
| **回合** | `cargo test --test turn_prefix_stability` | LLM 请求前缀稳定、seal 不改写、restore/reminder 只追加 |
| **持久化** | `cargo test --lib -- session::data::sqlite::ops::` | `persist_receipt_*`（含 delete 缺行接受） |
| **持久化** | `cargo test --lib -- final_immutability` | final 行不可改写；restart seal；seq 不复用 |
| **持久化** | `cargo test --lib -- types::death_list_gate::session_seq` | g1/g2/g4/g6 死亡清单 |
| **持久化** | `cargo test --test session_data_crud` | CRUD / cascade / op_id 幂等 / stale revision |
| **持久化** | `cargo test --test session_data_concurrency` | 并发 append + 同 revision 冲突 |
| **持久化** | `cargo test --test session_data_fault` | begin/commit 前后故障回滚与幂等 |
| **持久化** | `cargo test --test session_data_consistency` | change_log / hot transcript 一致 |
| **持久化** | `cargo test --test session_transcript` | transcript roundtrip + revert |
| **持久化** | `cargo test --test wire_seq_identity` | wire seq/cursor/compact/revert 身份 |
| **协议+serve** | `cargo test --lib -- client_protocol::connection::` | pending_ask + `ask_does_not_block_other_session_rpc` 等 8 |
| **协议+serve** | `cargo test --lib -- serve::run::` | case_a–d + loopback 绑定矩阵 6 |
| **协议+serve** | `cargo test --test client_protocol_project` | SessionEvent→wire 投影（compact/buffer/ask/turn…） |
| **协议+serve** | `cargo test --test client_protocol_permission` | Permission/AskUser WS bridge round-trip |
| **协议+serve** | `cargo test --test serve_bind_auth` | `bind_auth_matrix` |
| **协议+serve** | `cargo test --test subscribe_bash_hydration` | subscribe 水合 bash_jobs（linux 实跑 3） |

**可合并 lib 一次编跑（建议 CI）：**
```bash
cargo test --lib -- \
  'session::manager::child_session_tests|config::turn_guard|client_protocol::connection::|serve::run::|types::death_list_gate::session_seq|session::data::sqlite::ops::|final_immutability'
```

**可合并 integ 两组：**
```bash
cargo test \
  --test session_data_crud --test session_data_concurrency \
  --test session_data_fault --test session_data_consistency \
  --test session_transcript --test wire_seq_identity

cargo test \
  --test session_turn_entry --test pending_messages --test turn_prefix_stability \
  --test client_protocol_project --test client_protocol_permission \
  --test serve_bind_auth --test subscribe_bash_hydration
```

---

## B) 实跑结果（2026-10-12）

| 命令/批次 | 结果 | 日志 |
|---|---|---|
| `--lib -- session::manager::child_session_tests` | **22 pass / 0 fail** | `wp017-session/lib-child_session.log` |
| `--lib -- config::turn_guard` | **2 / 0** | `lib-turn_guard.log` |
| `--lib -- client_protocol::connection::` | **8 / 0**（含 `ask_does_not_block_other_session_rpc`） | `lib-connection.log` |
| `--lib -- serve::run::` | **6 / 0** | `lib-serve_run.log` |
| `--lib -- types::death_list_gate::session_seq` | **4 / 0**（g1/g2/g4/g6） | `lib-death_list_seq.log` |
| `--lib -- session::data::sqlite::ops::` | **4 / 0** | `lib-sqlite_ops.log` |
| `--lib -- final_immutability` | **8 / 0** | `lib-final_immutability.log` |
| 6× persist integ（crud/concurrency/fault/consistency/transcript/wire_seq） | **37 / 0**（8+5+5+4+4+11） | `integ-persist-core.log` |
| turn integ（turn_entry/pending/prefix） | **21 / 0**（4+5+12） | `integ-turn.log` |
| protocol/serve integ（project/permission/serve_bind/bash） | **37 / 0**（25+8+1+3） | `integ-protocol-serve.log` |

**必过合计：149 pass / 0 fail**（lib 54 + integ 95）。

环境备注：首次用系统 rustc 1.85 会直接拒编（icu≥1.86）；本实跑改用 box 上 rustc 1.99。单条 `--lib` filter 会反复 Finished～4min（增量脏）；合并 filter / 固定 `RUSTUP_HOME` 可缩短。

---

## C) 已知环境/门禁红（非本域产品回归）

| 命令 | 结果 | 说明 |
|---|---|---|
| `cargo test --test session_data_sql_death_list` | **1 pass / 1 fail** | `sessions_db_sql_and_rusqlite_stay_in_sqlite_module` 命中 `src/engines/session_search/**` 大量 `rusqlite` / `sessions.db join`，外加测试夹具与个别工具路径。**WP-012/WP-011 已知假红**；**不计入 Session 必过绿表**，禁当本域写路径破坏。日志：`integ-sql-death-list.log` |

另：`leftover_symbols_absent_from_repo` 在同 bin 内 **ok**。

---

## D) 禁区确认

- **未跑** WP-016 引擎产品测（无 `code_search` / 引擎产品 bin）
- **未跑** WP-004 symlink 复制测
- `session_data_sql_death_list` 仅单独跑一次做门禁记录，失败归因 session_search，不修、不松、不进必过绿

---

## 结论（给需求官）

Session / client_protocol / serve **必过集已钉 + 实跑全绿（149）**。唯一红是 sql death_list 对 session_search 的已知门禁假红，归 WP-011，非 session 主链回归。

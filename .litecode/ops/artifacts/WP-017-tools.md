# WP-017 · Tools + knowledge 必过清单与实跑

- owner 域：Tools（tools / permission / mcp / knowledge）
- 回报对象：需求官 · WP-017
- 盘点/实跑日：2026-10-12（HKT）
- 基线短哈希：`c6b24df`
- rustc（本机跑测）：`1.99.0`（`PATH` 指向 `/home/box/.rustup/toolchains/stable-x86_64-unknown-linux-gnu`；系统默认 `1.85.1` 会因 icu/ort/time MSRV 直接拒编）
- **禁区遵守：** 未跑 `optional_code_search` / 引擎产品测 / symlink 死亡清单
- **不 commit**

## 1) 必过清单（主链）

### Permission / Ask 桥
| 命令或路径 | 覆盖 | 含 ISSUE-030 |
|---|---|---|
| `cargo test --test permission_pipeline` | authorize / floor / Ask sink / Safe 矩阵 / schema 拒参 | authorize 门 ✓ |
| `cargo test --test client_protocol_permission` | WS permission 桥、request_id、abort≠deny | **AskUser questions map** + **Approval+free_text** round-trip ✓ |

关键测名（ISSUE-030）：
- `ws_bridge_ask_user_questions_round_trip_with_answers_map`
- `ws_bridge_approval_round_trip_with_free_text`

### MCP
| 命令 | 覆盖 |
|---|---|
| `cargo test --test mcp_integration` | initialize 单次、list/call、pool 复用/崩溃重建、hub block_on、hang 保持 pending |

### Subagent 工具管线（tools 边界）
| 命令 | 覆盖 |
|---|---|
| `cargo test --test subagent_tool_pipeline` | 并行 launch、后台 child、cancel 不误杀后台、panic 释放 session |
| `cargo test --test architecture_layers` | tool 层不 finish/join turn；runtime/protocol 不开 child session |

### Tools lib（ISSUE-030 + 主链）
| 过滤 | 覆盖 | ISSUE-030 |
|---|---|---|
| `tools::ask_user` | 多题 batch、legacy→q0、skip/missing sink | AskUser 产品路径 |
| `tools::custom` + `tools::custom_hub` | envelope/level/media、hub notice、**真进程后台** | **`background_real_process_begin_job_then_mailbox_notice`** |
| `tools::websearch` | 未配置/空 query/not_warmed 提示 | websearch 单元门 |
| `tools::webfetch::tests` | SSRF 公网/私网/scheme；`whatwg_*` **仅解析、不当 SSRF 门** | （021 已 closed；本批只保留） |
| `tools::bash_jobs_contract` | wait/kill/foreground detach 与 formatter 口径 | — |
| `tools::knowledge` | board/list/write 锁 / guide | — |

### Tool 核心 lib
| 过滤 | 覆盖 |
|---|---|
| `tool::executor` | cancel、partition、blob/local_file、schema 拒参、mailbox pending |
| `tool::registry` | binding / depth 排除 subagent / MCP 失败不广告 dummy |
| `tool::schema_validate` | required/type/enum/min_items |
| `tool::signal` | ok/warning/error 前缀与 hint 块 |
| `tool::write_lock` | 冲突/重入/workspace 粗锁 |

### Knowledge 库（非工具封装）
| 过滤 | 覆盖 |
|---|---|
| `knowledge::` | `spec_tests` / `index` / `view` / `status` / `mentions` / `fsref` / `corpus` / `reminder` |

### 明确排除（本 WP）
- `optional_code_search`、引擎产品测、symlink 复制/死亡清单（WP-011 / WP-016 / WP-004）
- `ssrf_whatwg_*`：**弱断言**（host_str 解析），勿当 SSRF 门（与 ISSUE-030 口径一致）

---

## 2) 实跑命令与结果

环境：`export PATH=/home/box/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin:$PATH`  
日志：`/tmp/wp017-integration.txt`、`/tmp/wp017-lib-tools-knowledge.txt`、`/tmp/wp017-lib-tool-core.txt`

### A. 集成必过（一次编过）

```bash
cargo test --test permission_pipeline \
  --test client_protocol_permission \
  --test mcp_integration \
  --test subagent_tool_pipeline \
  --test architecture_layers
```

| 包 | passed | failed | exit | 时刻（约） |
|---|---:|---:|---:|---|
| architecture_layers | 5 | 0 | 0 | 2026-10-12T12:27:11+08 → 12:30:29+08 |
| client_protocol_permission | 8 | 0 | 0 | （含 ISSUE-030 两 WS） |
| mcp_integration | 10 | 0 | 0 | |
| permission_pipeline | 16 | 0 | 0 | |
| subagent_tool_pipeline | 9 | 0 | 0 | |
| **合计** | **48** | **0** | **0** | 编译约 3m11s |

### B. tools + knowledge lib 过滤

```bash
cargo test --lib -- \
  tools::custom tools::websearch tools::webfetch::tests \
  tools::ask_user tools::custom_hub tools::bash_jobs_contract \
  knowledge::
```

| 结果 | 时刻 |
|---|---|
| **97 passed / 1 failed** · exit **101** · 过滤 1635 · ~1.54s（编 ~3m31s） | 2026-10-12T12:30:33+08 → 12:34:06+08 |

失败项（ISSUE-030 门）：
- `tools::custom::tests::background_real_process_begin_job_then_mailbox_notice`
- 现象：`stdin write failed: Broken pipe (os error 32)`（脚本 `sh -c 'echo …'` 先退出，stdin 写入竞态）

**单测复跑（flake 核查）：**
```bash
cargo test --lib -- tools::custom::tests::background_real_process_begin_job_then_mailbox_notice --exact
```
- retry1：1 passed · exit 0 · 2026-10-12T12:34+08
- retry2：1 passed · exit 0 · 同批稍后

→ **门有效、本机并行/批跑下偶发 Broken pipe；单独稳定绿。** 建议跟进：stdin 可空时跳过写，或测试命令改为读完 stdin 再 echo（不在本 WP 改码）。

模块计数（该次 98 过滤内）：knowledge 36 · ask_user 6 · bash_jobs 18 · custom 10 · custom_hub 5 · tools::knowledge 10 · webfetch 9 · websearch 4。

### C. tool 核心 lib 过滤

```bash
cargo test --lib -- \
  tool::executor tool::registry tool::signal \
  tool::schema_validate tool::write_lock
```

| 结果 | 时刻 |
|---|---|
| **55 passed / 0 failed** · exit **0** · 过滤 1678 · ~2.00s | 2026-10-12T12:34:11+08 → 12:38:16+08 |

拆分：executor 19 · registry 7 · schema_validate 18 · signal 6 · write_lock 5。

---

## 3) 汇总给需求官

| 桶 | 实跑 | 备注 |
|---|---|---|
| permission + Ask/Approval WS | **24/24**（16+8） | ISSUE-030 WS 两门绿 |
| mcp | **10/10** | |
| subagent pipeline + arch layers | **14/14**（9+5） | |
| tool 核心 lib | **55/55** | |
| tools+knowledge lib 过滤 | **97/98 批跑**；失败项 **2/2 单测复跑绿** | custom 真进程门 flake（Broken pipe） |
| **主链合计（不计 flake 复跑）** | **200 passed / 1 failed（可复现为竞态）** | 禁区未碰 |

### 假绿 / 缺口（不扩 scope）
1. **custom 后台真进程 flake**（stdin Broken pipe）— 记入本 artifact；是否升 issue 由需求官定。
2. `webfetch::whatwg_*` 仍非 SSRF 门（命名已标明）。
3. 未跑 optional_code_search / 引擎产品 / symlink。

### 建议必过脚本（给 CI/本机）
```bash
export PATH="/home/box/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin:$PATH"
cargo test --test permission_pipeline --test client_protocol_permission \
  --test mcp_integration --test subagent_tool_pipeline --test architecture_layers
cargo test --lib -- tools::custom tools::websearch tools::webfetch::tests \
  tools::ask_user tools::custom_hub tools::bash_jobs_contract knowledge::
cargo test --lib -- tool::executor tool::registry tool::signal \
  tool::schema_validate tool::write_lock
```

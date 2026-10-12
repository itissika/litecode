# WP-043 / WP-051 — IDE：核心覆盖核对 + 测试裁剪提案

- HEAD：`0c75d0a`（分支 `ops/team-framework`），日期：2026-10-12 20:16 HKT
- 执行：ide；审批：backend_mid。父包 WP-043。
- 工具链：直接 `cargo`（无 `PATH=/home/box/.cargo/bin` 前缀）；`HOME=/workspace/litecode/.tmp/home`；命令末 `</dev/null`；单套件 `timeout 600`。
- 编译：`timeout 3600 cargo test --no-run --test wp028_probe_s1_xdev_move --test wp028_probe_s2_create_race --test workspace_api --test workspace_path_consumer --test workspace_path_lap --test workspace_path_death_list --test terminal_bypass_death_list --test terminal_pty_contract --test terminal_ws_isolation --test workspace_process_contract --test workspace_hotswitch_death_list --test optional_lsp --lib` → Finished in 6.74s（无锁等待）。
- 运行：编译产物二进制（`CARGO_MANIFEST_DIR=$PWD`）；源码状态快照 `.tmp/wp051_srcstate_start.txt` / `.tmp/wp051_srcstate_run.txt`。
- **硬门禁**：不改引擎产品行为；不改 symlink 产品行为（WP-004）；覆盖阶段不改 death_list；裁剪只提案不删/不放宽落地；**不 commit**。

## 已批方案摘要（落盘）

两步硬串行：(1) 覆盖——核对 IDE 核心流程有效覆盖（037/038、path/terminal/process_contract、hotswitch death_list、WP-033/036 刚落），产出绿表+缺口；(2) 裁剪——列冗余/过时/重复及删/留/放宽提案交中层裁决，未过审前不删。对照 WP-021-backend §5 IDE 与 WP-022-ide T1–T6，重核现状。

---

## 1. 覆盖

### 1.1 绿表

| 套件 | P/F/I | 结果 | 命令（摘要） | 备注 |
|---|---|---|---|---|
| `wp028_probe_s1_xdev_move` | 1/0/0 | **ok** | `timeout 600 target/debug/deps/wp028_probe_s1_xdev_move-* --nocapture` | ISSUE-038 回归 |
| `wp028_probe_s2_create_race` | 2/0/0 | **ok** | `…/wp028_probe_s2_create_race-*` | ISSUE-037 回归（0.29s） |
| `workspace_api` | 12/0/0 | **ok** | `…/workspace_api-*` | CRUD / mkdir / rename / copy |
| `workspace_path_consumer` | 4/0/0 | **ok** | `…/workspace_path_consumer-*` | |
| `workspace_path_lap` | 5/0/0 | **ok** | `…/workspace_path_lap-*` | |
| `workspace_path_death_list` | 4/0/0 | **ok** | `…/workspace_path_death_list-*` | |
| `terminal_bypass_death_list` | 3/0/0 | **ok** | `…/terminal_bypass_death_list-*` | |
| `terminal_pty_contract` | 4/0/0 | **ok** | `…/terminal_pty_contract-*` | |
| `terminal_ws_isolation` | 3/0/0 | **ok** | `…/terminal_ws_isolation-*` | 2.81s |
| `workspace_process_contract` | 7/0/0 | **ok** | `…/workspace_process_contract-*` | 含 open 端点已删 |
| `workspace_hotswitch_death_list` | 0/1/0 → **2/0/0** | **先红后绿** | 初跑 FAILED；C1/C2 落地后 `cargo test --test workspace_hotswitch_death_list` 2 passed | ISSUE-007 已 closed；见 §6 |
| `optional_lsp` | 24/0/1 | **ok** | `…/optional_lsp-*`（37.14s） | `rust_analyzer_smoke` ignored；含 `hub_drop_kills_spawned_server_process` ok |
| lib `workspace::file_ops::` | 8/0/0 | **ok** | `…/litecode-* workspace::file_ops::` | atomic_create 等 |
| lib `workspace::service::` | 5/0/0 | **ok** | `…/litecode-* workspace::service::` | 含 symlink 拒拷**只跑不改** |

**源码 HEAD（编译与运行）**：`0c75d0a`。  
**合计（不含 hotswitch 假红）**：本域相关默认绿；唯一失败为 hotswitch death_list（预期，ISSUE-007）。

### 1.2 缺口

| # | 核心流程 | 判定 | 说明 |
|---|---|---|---|
| G1 | ISSUE-037 并发 create/write | **已覆盖** | 探针 2/2 绿；`atomic_create` 单元测在 file_ops 内 |
| G2 | ISSUE-038 跨盘 overwrite rename | **已覆盖** | 探针 1/1 绿（本机有 `/dev/shm`） |
| G3 | path / LAP / canonicalize | **已覆盖** | path_* 三套全绿 |
| G4 | terminal PTY / bypass / WS | **已覆盖** | 三套全绿；Tools 曾记 exit 码丢失（低）——产品缺口非本包范围，不修 |
| G5 | process_contract | **已覆盖** | 7/7 绿 |
| G6 | hotswitch death_list | **已放宽落地（绿）** | 初跑假红 8 处；C1/C2 收窄后 2/0/0；ISSUE-007 closed。见 §6 |
| G7 | LSP `kill_child` try_lock 跳过（S3） | **仍缺测** | `src/lsp/server.rs:771-776`：`try_lock` 失败则静默跳过 `start_kill`，且无 wait。`hub_drop_kills_spawned_server_process` 只覆盖锁可获的 happy path，**不覆盖锁竞争跳过**。公共 API 仍难构造；保持 suspect，本包不加钩子 |
| G8 | S4–S7（孙进程 / subreaper / code_search Drop / copy tmp） | **仍缺测** | 只读码；禁区外低优先级；本包不扩测 |
| G9 | symlink 拒拷 | **只跑不改** | `workspace::service::` 含相关测；WP-004 冻结 |
| G10 | 引擎 code_search / embed | **禁区跳过** | 未跑 `optional_code_search` / `code_search_ipc` |

**缺口条数（仍缺 / 假红 / 禁区记）**：G6 + G7 + G8（S4–S7 作一组）+ G10 = **4 类仍需关注**；G1–G5/G9 已覆盖。

### 1.3 WP-033/036 刚落核对

- WP-040（父 WP-036）落地的 037/038 修复与探针：本轮复跑全绿 → **未回退**。
- WP-033 无独立 IDE 子测套件；刚落行为通过上表 A/B/E 覆盖，无额外定位项。

---

## 2. 裁剪提案（只列，未删未放宽落地）

对照：WP-021-backend §5「IDE 基座（6）」；WP-022-ide §2 T1–T6；ISSUE-007 建议「按已删除符号禁」。本轮绿表重核后提案如下。

| # | 条目 | 提案 | 理由 | 对照旧清单 |
|---|---|---|---|---|
| C1 | `workspace_hotswitch_death_list` FORBIDDEN 子串 | **放宽已落地**（改匹配口径，不删清单） | ISSUE-007：按**已删除符号**禁；去掉/改写泛化 `reload_workspace` / `post_open` / `api/workspace/open`；本轮仍 8 处假红 | = WP-021 放宽；= WP-022 T1 |
| C2 | hotswitch 清单中的 `api/workspace/open` | **放宽/移交已落地** | 行为已由 `workspace_open_http_endpoint_is_gone`（process_contract，本轮绿）守；清单项重复致假红 | = WP-021 重复项；= WP-022 T3 |
| C3 | `workspace_path_death_list` 白名单自检 | **留**（可选并入 `no_bare_canonicalize` 前置断言） | 弱重复但防正则空转；本轮 4/4 绿 | = WP-021/WP-022 T2 |
| C4 | `rel_path_under_*`（path_consumer / path_lap） | **留或合并**到 `workspace_path_lap.rs` | 疑重复；未裁决不删；本轮相关套件绿 | = WP-021/WP-022 T5 |
| C5 | `terminal_bypass_death_list` | **留** | 承诺有效；本轮 3/3 绿 | = WP-021 保留；= WP-022 T4 |
| C6 | `workspace_api` CRUD | **留** | API 层覆盖；本轮 12/12 绿 | = WP-021 保留；= WP-022 T6 |
| C7 | `wp028_probe_s1` / `s2` | **留** | WP-040 正式回归；本轮全绿 | 新于 WP-021；WP-022 时为 ignore 探针 |
| C8 | `optional_lsp` 输出格式类断言 | **不裁本域** | 归属 Tools；本轮全绿（含原 1062 场景） | WP-028 曾交 Tools |
| C9 | `terminal_pty_contract` / `terminal_ws_isolation` | **留** | 本轮绿；旧清单未单列，补记保留 | 新于 WP-021 §5 短表 |
| C10 | lib `workspace::file_ops` / `service` | **留** | 支撑 037/038 与 symlink 拒拷只跑 | 新于 WP-021 §5 |

**裁剪提案条数**：10（C1–C10）。其中建议动作含「放宽」的仅 C1/C2（及 C4 可选合并）；**无一提案为「删」**；全部待 backend_mid 裁决后才可落地。

**待中层裁决（明确未落地）**：
1. 是否批准 C1/C2 放宽（ISSUE-007 修复包另开或并入后续 WP）？
2. C3/C4 合并是否做、谁做？
3. G7（S3）是否开测试钩子包，或继续 suspect？

---

## 3. 交叉审

- **ide → Tools**：本包无新增子进程。沿用 WP-028 对 `custom.rs` 的 X1/X2/X4；本包无新发现。
- **Tools → ide**：无新请求互审材料；沿用 WP-028 Tools 回填（terminal exit 码丢失 / pty Drop 有界阻塞）。本包未改 terminal/LSP 产品代码。

---

## 4. 门禁自检

- [x] 未改引擎产品行为（未碰 engines / embed / code_search 测试以外的产品）
- [x] 未改 symlink 产品行为（拒拷测试只跑）
- [x] 覆盖阶段未改任何 death_list 文件
- [x] 裁剪：仅 C1/C2 按中层裁决放宽落地；其余只提案未删
- [x] 未 commit
- [x] 产物路径：`.litecode/ops/artifacts/WP-043-ide.md`

---

## 5. 原始结果索引

- `.tmp/wp051_results/*.txt` — 各套件 stdout/stderr + EXIT
- `.tmp/wp051_compile.log` — `--no-run` 编译日志
- `.tmp/wp051_srcstate_*.txt` — HEAD/status 快照

---

## 6. 裁剪落地（reject→in_progress 后，backend_mid 裁决）

- 裁决时间：2026-10-12（HKT）；执行 ide；**不 commit**。
- **C1/C2 已落地**：`tests/workspace_hotswitch_death_list.rs`
  - 匹配：由裸 `str::contains` 改为路径子串 + **标识符边界**（`contains_forbidden`）。
  - 禁名单 **移除**泛化项：`reload_workspace`、`post_open`、`api/workspace/open`（open 交 `workspace_process_contract::workspace_open_http_endpoint_is_gone`）。
  - 禁名单 **保留**已删除符号：`commit_workspace_lock`、`WorkspaceOpenEvent`、`workspace/root_changed`、`on_workspace_open`、`workspace_root_changed`、`rebind_db_path`、`reload_root`、`workspace_open_mutex`、`resetForWorkspaceRoot`、`onRootChanged`、`WorkspaceRootChanged`。
  - 新增单测 `forbidden_match_uses_identifier_boundaries` 锁边界行为。
  - `git log -S`：公开史上无独立 `fn reload_workspace` 热切换定义可替换；三泛化项确认为误伤源，按裁决删除而非改名。
- **复跑**：`HOME=/workspace/litecode/.tmp/home timeout 600 cargo test --test workspace_hotswitch_death_list -- --nocapture </dev/null` → **ok. 2 passed; 0 failed; 0 ignored**（约 0.80s + 编译 20s）。日志：`.tmp/wp051_results/hotswitch_rerun.txt`。
- **ISSUE-007**：status → `closed`；history 记 WP-051 落地。
- **未落地（仍为提案）**：C3/C4 合并暂缓；C5–C10 留。G7/G8 继续 suspect，本包不开钩子。
- **禁区**：未碰引擎产品、symlink 产品行为；未 commit。

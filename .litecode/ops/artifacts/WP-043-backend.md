# WP-043 · 后端汇总（后端中层）

- 基线：分支 `ops/team-framework`，HEAD `0c75d0a` + 未提交工作区（测裁剪落地未 commit）。中层未改 `src/` 产品行为；未 commit。
- 工具链：`rustc 1.99.0`；本机直接 `cargo`（无 PATH 前缀）。共享 target。
- 父：WP-042（cover-then-prune）。子包 WP-047..051 均已 **done**。
- 时间：2026-10-12 20:25 HKT。
- 禁区遵守：未碰引擎产品、symlink 产品行为、ISSUE-032 / ISSUE-034。

## 1. 总览

| 子包 | 域 | 状态 | 覆盖 | 裁剪落地 | 交叉审 | 关闭/登记 |
|---|---|---|---|---|---|---|
| WP-047 | Session | done | F1–F6 绿；G5-wire / G-pending 软缺口 | T1–T4 落地；T5–T8 不动 | 与 Runtime/Context 只读一致 | ISSUE-041 owner=session（假红登记，未修） |
| WP-048 | Runtime | done | C1–C8 绿；无阻断缺口 | **无落地**（全 keep/defer） | Session：同意 F5 分层与 T5 双留 | ISSUE-036 / G5-wire 另跟 |
| WP-049 | 上下文 | done | C1–C8 全绿；ISSUE-035 回归绿 | P1–P4 删；P6/P7 合并；P5/P8–P12 留 | Session：有条件接受（G1 pending-cache 非阻断） | — |
| WP-050 | Tools | done | C1–C12 复核绿（部分环境软缺口） | T2–T6/T8 放宽；T9 删；T1/T7/T10 留 | 沿用 WP-028/036 ↔ IDE 进程结论 | ISSUE-039/040 已 closed（本包只复核） |
| WP-051 | IDE | done | 核心套件绿；hotswitch 先红后绿 | C1/C2 放宽落地；C3–C10 留 | ↔ Tools：无新子进程；沿用 WP-028 | **ISSUE-007 closed** |

## 2. 执行模式

硬串行：各域先覆盖（绿表+缺口）→ 提案交中层裁决 → 获批项才落地 → 复跑绿 → 中层 close。单条裁剪不逐项报 GM。中层不写产品/测试代码。

## 3. 分域摘要

### Session（WP-047）

- **覆盖**：协议生命周期、permission wire、list/filter、compact×DB 交叉、重连分层、telemetry 等绿。
- **缺口**：G5-wire（JSON-RPC 全链路）仍开；pending-cache 同回合重复为已知软缺口。
- **裁剪落地**：T1 telemetry 合并；T2 留完整 permission wire、删两条快照；T3 empty_filter 表驱动；T4 `sleep(50ms)`→notify/轮询。T5 双留 calibers+e2e；T6–T8/N* 不落地。

### Runtime（WP-048）

- **覆盖**：agent_loop / auto·manual compact 失败 / 取消→Cancelled / 重连停回合 / compaction_failed 投影 / pending auto-turn / ISSUE-035 交界读码，全绿。
- **裁剪**：T1 helper 可选暂缓；T2 留 calibers；T3 sql death_list 跟 ISSUE-041；T4 取消三态留。**本包零删测。**

### 上下文（WP-049）

- **覆盖**：compact DB 真相、fold 重载、reminder/budget/keep_recent、跨域行为门全绿。
- **裁剪落地**：删 engine 四条与 sources 重复测（P1–P4）；budget / keep_recent 各合并一条（P6/P7）；death_list 与跨域门保留。
- **交叉审（Session）**：ISSUE-035 口径一致；G1 pending-cache **有条件接受、非阻断**。

### Tools（WP-050）

- **覆盖**：custom/MCP 生命周期（039/040 已闭只复核）、ask_user/plan/bash/permission/knowledge/webfetch/lsp 等绿；Windows Job / 集成 warmup 超时记软缺口。
- **裁剪落地**：文案/时序放宽 T2–T6/T8；删 webfetch 纯第三方样板 T9；保留 T1/T7/T10。

### IDE（WP-051）

- **覆盖**：037/038 探针、path/terminal/process_contract、optional_lsp、lib file_ops/service 绿；G7 LSP `try_lock` 跳过仍 suspect；引擎禁区未跑。
- **裁剪落地（C1/C2）**：`workspace_hotswitch_death_list` 改标识符边界匹配；移除泛化 `reload_workspace` / `post_open` / `api/workspace/open`；复跑 **2 passed**；**ISSUE-007 → closed**。C3–C10 留。

## 4. 交叉审汇总

| 对 | 结论 |
|---|---|
| Session ↔ Context（WP-049） | 有条件接受：ISSUE-035 一致；G1 pending-cache 非阻断 |
| Session ↔ Runtime（WP-048） | 同意 F5 分层；T5 calibers+e2e 双留；G5-wire 仍开 |
| Tools ↔ IDE | 本波无新子进程交叉材料；沿用 WP-028/036 生命周期结论 |

## 5. Issue / 跟进（不挡本包）

| 项 | 状态 | 说明 |
|---|---|---|
| ISSUE-007 | **closed** | hotswitch death_list 放宽（WP-051） |
| ISSUE-041 | open（owner=session） | sql death_list×session_search 假红；本包只登记 |
| G5-wire | 软缺口 | Session/Runtime JSON-RPC 全链路另包 |
| G1 pending-cache | 软缺口 | Context/Session；非阻断 |
| IDE G7/G8 | suspect | LSP 锁竞争跳过等；本包不开钩子 |
| ISSUE-032 / 034 | 禁区 | 本波未碰 |

## 6. 门禁自检

- [x] 五域 cover → mid 裁决 → 落地/暂缓 → close
- [x] 未改引擎产品 / symlink 产品行为
- [x] 未修 ISSUE-032 / ISSUE-034
- [x] 中层未写 `src/` / `tests/`；未 commit
- [x] 产物：`.litecode/ops/artifacts/WP-043-{session,runtime,context,tools,ide,backend}.md`

## 7. 请 GM 验收

父包 WP-043 完成 cover-then-prune。请 `close --to done`（或 reject 并说明）。前端 WP-044 已关；本包为后端侧收口。

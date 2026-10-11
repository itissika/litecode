# Batch 3 · client-ux（web / 桌面交互）

负责人：前端 · 基线 `075308b` · 日期 2026-10-12（HKT）  
范围：`ux-design` / `ux` / `ux-interaction` / `client-projection` + pending `layout-ssot` / `ask-surface` / `theme-tokens` / `knowledge-client-view`。  
**不含** `connection-ask-registry` / `serve-bind-auth` 主核实（Session → `BATCH-3-client-ux-protocol.md`）；本切片对 DRAFT-CUX-1 做联查并定稿。

## 1. 节点对照表

| key | 状态 | 结论 | 证据（文件 + 符号/测试） |
|---|---|---|---|
| `ux-design` | enabled | **一致，无实质漂移** | 主张为理念层；投影纪律由 `client-projection`+死亡清单测试守护（`web/src/api/r4-death-list.test.ts` 等）；Ask 表面见 `PermissionCard`；美学由 `theme-tokens` 承接。未改 enabled 主张。 |
| `ux` | enabled | **一致，无实质漂移** | 面板分区与「一切皆投影 / 会话独立于 tab / 知识一等公民 / 视觉 token」与 `web/src/dockview/workbench/*`、`knowledgeStore`、`tokens.css` 一致。挂树含 pending 子节点 → `inactive_target` warning 预期内。`serve-bind-auth` 归 Session。 |
| `ux-interaction` | enabled | **一致，无实质漂移** | 三层交互；权限卡为面板浮层非全屏；回执经 `turnStore.grantPermission` → `agent/permission`。契约展开 → `ask-surface`。 |
| `client-projection` | enabled | **一致，无实质漂移** | `messageStore` 按 seq 镜像、乐观气泡无永久身份（`web/src/stores/messageStore.ts`）；死亡清单 `web/src/api/*-death-list.test.ts`。契约展开 → `connection-ask-registry`（Session 主核实）。**ISSUE-001** 仍为 gate 守护缺口（测试未跟上 PermissionAsk 新字段），不否定主张本身。 |
| `layout-ssot` | pending | **一致，无实质漂移** | `openPanel` / `movePanelToMain` / `startWorkbench`（`web/src/dockview/workbench/commands.ts`、`lifecycle.ts`）；守护 `placement.test.ts`。`editorStore.tabs` 持**内容**（path/dirty/bytes），面板「在不在/在哪」经 workbench/`hasPanel`/`onPanelRemoved` 跟从 Dockview，非第二布局真源。 |
| `ask-surface` | pending | **一致，无实质漂移** | `PermissionRequest.kind`: permission\|approval\|ask_user（`web/src/api/types.ts`）；`PermissionCard` 分发三卡；`AskUserCard` 多题 + `answers` map；出站 `agent/permission`（`turnStore.grantPermission`）。多 session 闸见 ISSUE-019（已 closed）。 |
| `theme-tokens` | pending | **一致，无实质漂移** | `web/src/theme/tokens.css` 声明唯一 `--_dk-*`；`composerCard.ts` 导出 `glassFill`。Ask/磨砂卡消费 token，非第二主题系统。 |
| `knowledge-client-view` | pending | **一致，无实质漂移** | `fetchKnowledgeSnapshot` + `pullSnapshot` + `acceptsKnowledgeVersion`；`files.ts` 已删；打开面板走 `openKnowledgeGraphPanel`→`openPanel`，无写盘副作用。本地 `markers` 仅预览。权威互引见 `knowledge-ref-authority`。 |

**统计：** 一致 8 / 不能扩写 0。本域 **无 B2**（无 core 节点结论 B）。

## 2. 正式 issue

| id | grade | layer | status | 由草案 | 摘要 | owner |
|---|---|---|---|---|---|---|
| ISSUE-008 | drift | gate | open | DRAFT-CUX-2 | PermissionModal 测试 `justify-start` 过期 | 前端 |
| ISSUE-019 | bug | product | **closed** | DRAFT-CUX-1 | 多 session Ask 串行化 WS：**基线前已修**（74f2bb1）；测试 `ask_does_not_block_other_session_rpc`；前端无第二闸 | Session（协议）+ 前端联查 |
| ISSUE-001 | drift | gate | open（种子） | DRAFT-CUX-3 | `client_protocol` 权限投影测试未跟上 Ask 字段 | Session（前端联查确认仍缺字段） |

机读：`.litecode/ops/issues/ISSUE-008.json`、`ISSUE-019.json`。

### DRAFT-CUX-1 / ISSUE-019 联查结论（给 Session）

- Andy 口径：「一 session 只管自己的 Ask；整条 WS 串行 = 错」。
- **实现已符合口径**（基线 `075308b`）：
  - `run_session_loop` 注释与路径：Ask 登记在 `pending_asks`，**不**再嵌套 select 把其它 session RPC park 进 deferred；`JsonRpc` 分支注释 “Human interactive RPC stays live during Ask (any session)”。
  - 回归：`ask_does_not_block_other_session_rpc`。
  - 前端：`pendingPermission` 在 `turnStore.byId[sessionId]`；`start()` 只看本 session `runState`，不读其它 session 的卡。
- Session 侧请在 `BATCH-3-client-ux-protocol.md` 将 `connection-ask-registry` 标为**一致**，并引用同一测试；**无需再为 CUX-1 改主环**（除非发现测试未覆盖的 `agent/prompt` 特例——当前读码未见 Ask 期间对异会话 JsonRpc 的 park）。

## 3. 待删 / 待改非知识 md（仅本域撞车，先标不删）

| 路径 | 说明 |
|---|---|
| （无强制待删） | `desktop/README.md` / `CHANGELOG.md` / `README.zh-CN.md` 与瘦包叙事一致，属发布域文档，见 release-desktop artifact。 |
| `web` 内无第二套「权限协议」md 与 ask-surface 抢主张。 | |

## 4. 互引补全（不改主张）

- `knowledge-client-view` ←已→ `knowledge-ref-authority`（对端已有；本节点补 `相关：`）。
- `ask-surface` 补 `相关：[@ key="connection-ask-registry"]`（与 Session 节点对称）。
- enabled 节点未改主张句；仅 pending 补短核实结论块（admission §7.2）。

## 5. 阻断声明

**无。** 无 arch；无 core 结论 B；ISSUE-019 已 closed 且不触达 B1–B5。

## 6. kb_check

对本域上表节点逐个 `--node`：`errors=0`。enabled 父节点→pending 子节点的 `inactive_target` warning 预期内（KR-NODE-3）。

## 7. 本轮未改代码

核实为主；未修 ISSUE-008（gate，先记 issue）；未改 enabled 主张含义。

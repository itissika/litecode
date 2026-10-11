# Batch 2 · 跨域对齐报告（GM）

- 日期：2026-10-11（HKT）
- 工作区 HEAD：`075308b`（未 commit / 未 push；对齐仅读检 + 本报告）
- 校验：`node .litecode/ops/tools/kb_check.mjs --json` → tools_version=7
- 存量基线 stock_ref：`e583ff2`（准入文档）

## 1. 总体数字

| 项 | 基线（Batch 2 前） | 现在 |
|---|---|---|
| 节点 | 36 | **78**（存量 36 + 新建 42） |
| enabled / pending | 26 / 10 | 26 / **52**（新建 42 全 pending；存量 tools 子契约 10 仍 pending） |
| 引用 | 205 | 451 |
| **error** | 0 | **0** |
| warning | 119 | **53** |
| `dup_primary_suspect` | 32（16 对） | **0** |

新建 42 个节点均带 `domain` + `基线：075308b`；无同名 key 冲突；无非法 domain id。

### 按负责人 × 域的新建节点

| 负责人 | 域 | 新建 keys |
|---|---|---|
| Session | harness-core | session-write-path, session-log-envelope, turn-reservation |
| Session | client-ux | serve-bind-auth, connection-ask-registry |
| Runtime | harness-core | item-authority, agent-loop, stream-projection, turn-llm-binding, platform-knobs |
| Runtime | model-config | provider-catalog, llm-provider-surface, llm-stream-contract |
| 上下文 | harness-core | reminder-log, context-budget, context-compact, context-keep-recent, context-media |
| IDE 基座 | ide-substrate | workspace-sandbox, workspace-watcher, terminal-pty, lsp-hub-lifecycle, process-lifetime |
| Tools | tools-permission | mcp-connection-pool, custom-tool-hub, webfetch-ssrf, websearch |
| Tools | knowledge | knowledge, knowledge-ref-authority, knowledge-parse, knowledge-index, knowledge-ref-resolve, knowledge-tool |
| 前端 | client-ux | layout-ssot, ask-surface, theme-tokens, knowledge-client-view |
| 前端 | release-desktop | release-design, desktop-shell, slim-packaging, component-distribution, bundle-resolve |

合计 42；与 `git` 相对 `e583ff2` 的新增 knowledge 文件清单一致。

### Artifacts 清单（本对齐已读）

- `BATCH-2-harness-core-session.md` / `BATCH-2-client-ux-protocol.md`（Session）
- `BATCH-2-harness-core-runtime.md` / `BATCH-2-model-config.md`（Runtime）
- `BATCH-2-harness-core-context.md`（上下文）
- `BATCH-2-ide-substrate.md`（IDE 基座）
- `BATCH-2-tools-permission.md` / `BATCH-2-knowledge.md`（Tools）
- `BATCH-2-client-ux-web.md` / `BATCH-2-release-desktop.md`（前端）

## 2. kb_check 结果

- **errors = 0**（无 missing_symbol / filename_mismatch / dangling_relation / 新节点违规）
- warnings 按 code：

| code | 数 | 说明 |
|---|---|---|
| `inactive_target` | 28 | enabled 父节点挂 pending 子节点；准入允许，Batch 3 升降后再消 |
| `domain_missing` | 9 | 全部为存量未回填（见下） |
| `baseline_missing` | 9 | 同上 9 个存量 |
| `parent_multiple` | 5 | 存量：engine-lifecycle / retrieval-contract / featured-tools / agent-harness-philosophy / workspace-shared-substrate |
| `parent_missing` | 1 | 仅根节点 `why`（合理；工具可后续豁免） |
| `tag_unknown` | 1 | `why`「理念的落点：」（存量） |
| **`dup_primary_suspect`** | **0** | 相对基线 32 条已清零 |

### Tools 消 dup 声明核对

基线（`kb_check-baseline.md`）16 对集中在 `contracts/tools/*`，证据多为 `fn run_tool` / `execute_batch_cancellable` / Custom·MCP `execute`。  
当前全库 `dup_primary_suspect` **0**，与 Tools artifact「本域 dup=0」一致。**声明成立。**

## 3. 跨域主张碰撞检查

| 对 | 判定 | 说明 |
|---|---|---|
| 同 key 两处 | **无** | 78 文件 / 78 唯一 key |
| client-projection ↔ session-model | **分层 OK** | 投影纪律 vs 会话真源；证据分属 client_protocol/web vs session sqlite。互缺 `相关：`（见开放项） |
| knowledge-client-view ↔ knowledge-* | **分层 OK** | 客户端只投影快照 vs KR-INV 磁盘权威。authority → client-view 已有 `相关`；反向缺 |
| agent-loop ↔ tool-pipeline ↔ turn-reservation | **分层 OK** | L0 控制流 / 工具入口授权 / session 忙碌预占。并列挂在 turn-lifecycle 下（Session 已问 GM）：**确认 OK**，不抢主张 |
| ask-surface ↔ connection-ask-registry | **分层 OK** | UI kind/卡片 vs 连接侧 pending 表；registry → surface 已引用 |
| bundle-resolve ↔ engines embed | **分层 OK** | 壳侧路径纪律；Rust `find_bundled_model_dir` 明确留给 ide-substrate |
| agent-loop ↔ tools 子契约（共享 `fn run` 历史） | **可接受** | Runtime 已说明非主节点重复；当前无 dup 警告。Batch 3 可让 tools 侧继续改用专属符号减轻启发式 |

**本轮未改任何 knowledge 节点文件**（无可机械消歧的错误 domain / 同符号唯一证据双主节点）。

## 4. 存量未回填 domain / 基线（9）

| key | 建议 domain（供 Batch 3 回填，非本轮改） |
|---|---|
| why | （根；可豁免 parent） |
| product-brief | （产品总览；跨域） |
| agent-design | harness-core |
| agent-harness-philosophy | harness-core |
| config / config-commit | model-config |
| featured-tools / tools | tools-permission |
| subagent-lifecycle | harness-core（与 Tools 交叉处只引用） |

Runtime 已轻改 `config` / `agent-harness-philosophy` 的挂树，但未补 domain——记入开放项，不在本轮代填。

## 5. Issue 草稿总表

| id / 草案 | grade | layer | 域 / 核心节点 | 负责人 | 来源 artifact |
|---|---|---|---|---|---|
| ISSUE-001 | drift | gate | client-ux / client-projection | Session（+前端联查） | client-ux-protocol / client-ux-web |
| ISSUE-002 | drift | gate | ide-substrate / retrieval-contract | IDE 基座 | ide-substrate |
| ISSUE-003 | drift | product | harness-core / session-model | Session | harness-core-session |
| ISSUE-004 | suspect | product | ide / lsp（旁听 tools） | IDE 基座 | ide-substrate |
| ISSUE-005 | drift | product | model-config | Runtime | model-config |
| ISSUE-006 | drift | gate | （SQL 死亡清单） | 知晓即可 | harness-core-runtime |
| ISSUE-007 | drift | gate | ide-substrate / workspace-* | IDE 基座 | ide-substrate |
| ISSUE-008 | drift | gate | client-ux | 前端 | client-ux-protocol / web |
| ISSUE-009 | suspect | env | ide-substrate / engine-lifecycle | IDE 基座 | ide-substrate |
| ISSUE-010 | bug | product | ide-substrate / workspace-sandbox | IDE 基座 | ide-substrate |
| ISSUE-011 | drift | gate | tools / webfetch-ssrf | Tools | tools-permission |
| ISSUE-012 | bug | product | tools / webfetch-ssrf（A6） | Tools | tools-permission |
| DRAFT-CUX-1 | suspect | product | client-ux（多 session Ask 串行化 WS） | 前端→Session 确认 | client-ux-web |
| DRAFT-CUX-2 | drift | gate | = ISSUE-008 | 前端 | client-ux-web |
| DRAFT-CUX-3 | drift | gate | = ISSUE-001 | Session | client-ux-web |
| DRAFT-CTX-001 | suspect | product | harness-core / context-view | 上下文 | harness-core-context |
| DRAFT-CTX-002 | suspect | product | harness-core / reminder-log | 上下文 | harness-core-context |
| DRAFT-CTX-003 | drift | gate | harness-core（死亡清单脆弱） | 上下文 | harness-core-context |
| DRAFT-HC-R1 | suspect | gate | harness-core / platform-knobs | Runtime | harness-core-runtime |
| DRAFT-MC-1 | suspect | product | model-config / config-commit | Runtime | model-config |
| DRAFT-MC-2 | suspect | gate | model-config / llm reconnect | Runtime | model-config |
| DRAFT-REL-1 | suspect | — | release-desktop（Release 缺组件资产） | 前端 | release-desktop |
| DRAFT-REL-2 | drift | product | release-desktop（路径前缀泄露） | 前端 | release-desktop |
| DRAFT-REL-3 | — | — | updater 单独立项 | 前端 | release-desktop |
| KN-B2-01 | suspect | — | knowledge（证据/依据双名） | Tools / GM | knowledge |
| KN-B2-02 | — | — | 无 bug（占位说明） | Tools | knowledge |
| TP-B2-01 | suspect | — | tools / custom-tool-hub | Tools | tools-permission |
| TP-B2-02 | suspect | — | tools / mcp-connection-pool | Tools | tools-permission |

**阻断：** 当前无 arch；核心 product 级 open 为 ISSUE-003 + ISSUE-010 + ISSUE-012（webfetch）等，**未触达 B1–B5**。ISSUE-012 为确认 bug（A6 局部），按既定策略不阻断整批，Batch 3 优先修。

## 6. 本轮动作

| 动作 | 结果 |
|---|---|
| 跑全库 kb_check | errors=0，warnings=53，dup=0 |
| 清点 10 份 BATCH-2 artifact + 42 新节点归属 | 全覆盖、无遗漏 key |
| 跨域主张抽检（上表 6 组） | 无需改节点正文 |
| knowledge 文件编辑 | **无** |
| append_log | decision：Batch 2 对齐完成，建议可进 Batch 3（见 log） |

## 7. 开放项（需 Andy / GM，不阻断 Batch 3 启动）

1. **互引补全（建议 Batch 3 开头各负责人自补 `相关：`，不改主张）：**  
   client-projection ↔ session-model；knowledge-client-view → knowledge-ref-authority；agent-loop ↔ tool-pipeline / turn-reservation。
2. **存量 9 节点 domain/基线回填**（上表）；`why` parent 豁免是否进 kb_check。
3. **KN-B2-01：** 准入模板「证据：」与标签「依据：」是否收敛文案（工具已双接受）。
4. **DRAFT-CUX-1：** 多 session Ask 串行化是否改主环——需 Session 在 Batch 3 定性（suspect）。
5. **turn-lifecycle 下并列：** turn-reservation（Session）与 agent-loop / stream-projection / turn-llm-binding（Runtime）——**GM 确认保留并列**。

## 8. 结论

**建议：可以进入 Batch 3。**

理由：全库 error=0；新建目录覆盖 7 个产品域（含原空的 knowledge / release-desktop）；基线 dup 已清；跨域主节点无硬冲突；issue 草稿已汇表且无阻断触发。Batch 3 按域核实实现↔承诺，并落盘正式 issue。

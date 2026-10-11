# 知识节点准入标准（Batch 1 团队上下文）

版本：v1（2026-10-11，总经理起草，待 Andy 审）。基线提交：`e583ff2`（ops/team-framework）。
适用：本轮“全域知识体系”整批工作（Batch 2 钉目录、Batch 3 核实现）。issue 分级与阻断见同目录 `issue-grading.md`。

## 0. 知识库语法（以仓库现状为准，不另造）

权威：`docs/knowledge-references.md`（规范 v2，规则编号 KR-*）；解析器 `src/knowledge/document.rs`；索引与状态 `src/knowledge/index.rs`；契约表 `tests/fixtures/knowledge_refs/cases.json`，锁定测试 `src/knowledge/spec_tests.rs`。

- 知识根：`knowledge/`（KR-NODE-5），递归收所有 `.md`；跳过 `.` 开头的文件/目录。
- 声明块：文件开头的 ```` ```node ```` 围栏，字段行 `name : value`。识别的字段：`node`（键）、`status`（`enabled`/`disabled`/`pending`，缺省按 enabled 解析）、`summary`、`x`/`y`/`w`/`h`（画布）。其余字段行原样保存在 `extras` 并写回（`document.rs` 的 `parse_knowledge_markdown`）。`ref :` 行被忽略（KR-SYN-7）。
- 文件名主干必须等于键，否则 `filename_mismatch`（KR-DECL-1）。键规则见 KR-SYN-1。
- 引用只在声明块之后的正文里，四种固定写法（KR-SYN-1..5）：
  - 节点：`[@ key="K"]`
  - 文件：`[@ file="src/a.rs"]`
  - 符号：`[@ file="src/a.rs" symbol="impl Store › fn save"]`（链用 ` › `，**不带** `lines`，KR-SYM-6；HEAD 漂移会报 `symbol_drift`，KR-DRIFT-1）
  - 行范围：`[@ file="src/a.rs" lines="12-30"]`（不追踪漂移，KR-RANGE-1）
- 代码块/行内代码里的写法不是引用（KR-SYN-6）。属性顺序必须 `file → symbol → lines`，否则整段是普通文本（KR-SYN-2）——写错不会报错，只会“悄悄不算引用”，务必自查。
- 关系标签是**正文惯例**，不是语法：现有节点用 `上层：`（23 处）、`依据：`（12）、`相关：`（8）、`契约展开：`（6）、`父节点：`（5）、`框架展开：`（4）、`向下展开：`（2）。
- 状态与告警：`enabled` 节点引用 `pending`/`disabled` 目标报 `inactive_target`（警告，KR-NODE-3）；非 enabled 节点引用谁都不报。

## 1. 现状盘点（基线 `e583ff2`）

36 个节点：27 个 `enabled`，9 个 `pending`（全部在 `knowledge/contracts/tools/`），0 个 `disabled`。
层次：`why` → `product-brief` → `design/*`（agent/engine/ux）→ `framework/*`（config/context/session/tools/featured-tools/ux）+ `harness/*` → `contracts/*`（11 个 enabled 契约）→ `contracts/tools/*`（9 个 pending 子契约）。
证据形态：enabled 节点只用文件级 `依据：[@ file=…]`；pending 工具子契约用符号级引用（每节点 5–6 个），是本轮节点的样板。

## 2. 域与源码作用域

域取自 `.litecode/ops/board.json`。board 里只有知识基线，没有源码路径；下表源码映射由总经理补定，Batch 2 各负责人可提出调整，经总经理确认后改本表。

| 域 id | 现有知识基线 | 源码作用域（主） | 覆盖情况 |
|---|---|---|---|
| harness-core | contracts/{turn-lifecycle,session-model,context-view,subagent-lifecycle}；framework/{session,context}；design/agent-design | `src/agent` `src/runtime` `src/session` `src/context_pipeline` `src/reminder` `src/types` `src/authority.rs` `src/platform_knobs.rs` `src/tools/subagent` | 已覆盖（框架+4 契约）；reminder、types/authority 无节点 |
| tools-permission | contracts/tool-*（4 enabled）+ contracts/tools/*（9 pending）；framework/{tools,featured-tools} | `src/tool` `src/tools`（subagent、code_search/session_search/lsp/knowledge 工具的引擎侧除外）`src/permission` `src/mcp` `src/optional` | 覆盖最深；mcp、custom_hub、webfetch/websearch 无专属节点 |
| ide-substrate | harness/workspace-shared-substrate；contracts/{engine-lifecycle,retrieval-contract}；design/engine-design | `src/workspace` `src/engines` `src/lsp` `src/terminal` `src/ide_base.rs` `src/proc_lifetime.rs` | 部分：workspace 沙箱/watcher、LSP、终端 PTY、进程寿命无契约节点 |
| knowledge | （无节点） | `src/knowledge` `src/tools/knowledge.rs`；规范 `docs/knowledge-references.md` | **未覆盖**：规范只在 docs，知识库里没有自描述节点 |
| model-config | framework/config；contracts/config-commit | `src/config` `src/provider_catalog` `src/llm` | 部分：llm 流式/provider 适配无节点 |
| client-ux | framework/ux；design/ux-design；ux/ux-interaction；contracts/client-projection | `src/client_protocol` `src/serve` `src/cli` `web/src` | 部分：serve 认证/绑定、ws 协议无契约节点 |
| release-desktop | （无节点） | `desktop/src` `scripts/` `.github/workflows/` `build.rs` `src/version.rs` `src/telemetry` | **未覆盖** |
| team-ops | — | `.litecode/ops/` | 不产出产品知识节点 |

一条规则：源码文件只能归一个域。上表有歧义时以“该文件里定义的类型属于哪个契约的主节点”为准。

## 3. 准入依据（满足至少一条，并能写出一行判定）

| 编号 | 依据 | 一行判定（答不出就不准入） |
|---|---|---|
| A1 | 模块唯一关键不变量 | “这个模块无论怎么改，哪一句话为假就算坏了？”——能写成一句可证伪断言，且有测试或代码点守护/应当守护。 |
| A2 | 生命周期 / 状态机 | 有 ≥2 个具名状态和受限迁移；能列出“不允许的迁移”或“谁拥有迁移权”。 |
| A3 | 数据与所有权流向闭环 | 能说出某份数据的唯一真源、唯一写者、派生物及其重建方式；派生物不得冒充真源。 |
| A4 | 跨模块契约 | 接口/协议/事件被 ≥2 个域消费，且双方对字段语义或顺序有依赖（如 `call_id` 配对、`(session, seq)` 身份）。 |
| A5 | 失败与恢复语义 | 能说清提交边界在哪：失败前后哪些状态可见、重试是否幂等、取消/超时/崩溃后留下什么。 |
| A6 | 安全与权限边界 | 存在一道“不可放宽”的门（沙箱、floor、SSRF、认证），能说出门在哪一处唯一实现、绕过算什么。 |
| A7 | 架构决策与禁令（仓库特有） | 已被 ADR（`docs/adr/*`）或死亡清单测试（`tests/*_death_list.rs`、`src/types/death_list_gate.rs`）钉住的“只许一处 / 禁止第二套”。 |

A7 是对总经理方向的补充：本仓已有大量以死亡清单测试表达的架构禁令，它们本身就是候选不变量的证据。

## 4. 排除项（出现即退回）

- 实现细节：算法步骤、缓存策略、具体阈值数字（除非阈值本身是对外承诺，如 KR-READ-1 的 10 MiB）。
- 函数清单 / 字段清单 / 文件清单：读代码即可得到的枚举。
- 复述：能从一个符号的签名或 doc comment 直接读出的内容。
- 未来计划、待办、愿景（写进 demand 板，不进节点）。
- 只描述单个私有函数、无跨模块后果的点。
- 与另一个节点主张重复的内容（见第 5 节）。

自测：把节点正文里的源码引用全部删掉，剩下的话是否仍是一条“承诺”。若剩下的是“它调用了 X 然后 Y”，退回。

## 5. 主节点与引用规则

- **一个契约只有一个主节点。** 主节点写主张与证据；其他节点只用 `[@ key="主节点键"]` 引用，**不复述其要点**（最多半句指向语，如“授权见 [@ key="tool-authorization"]”）。
- **主节点归属**：契约主要实现所在源码文件属于哪个域（第 2 节），主节点就归哪个域。跨域消费方用 `相关：` 引用。
- **标签固定为五个**（沿用现有惯例，新节点不得再造标签）：
  - `上层：[@ key="…"]` —— 必填，恰好一个上层（多个上层的现有节点保持原状）。
  - `父节点：[@ key="…"]` —— 仅用于同一契约的拆分子节点（如 `contracts/tools/*`），与 `上层：` 二选一。
  - `契约展开：` / `框架展开：` / `向下展开：` —— 只写在上层节点里指向下层。
  - `相关：[@ key="…"]` —— 横向引用，指向别域主节点。
  - `依据：` —— 源码证据（见第 6 节）。
- **新节点声明块增加一行 `domain : <域 id>`**（落在 `extras`，解析器原样保留写回），用于归属统计和去重检查。存量 enabled 节点不加（需 Andy 批准）。
- 新节点 `status : pending`。它可以引用 enabled 节点；enabled 节点向下引用它需要改 enabled 节点，按第 8 节走 Andy 审批，审批前不挂接，靠新节点的 `上层：` 单向挂树。
- 键：英文小写短横线，等于文件名主干；目录沿用 `knowledge/{design,framework,harness,contracts,ux}/`，同一契约拆分子节点放子目录（参照 `contracts/tools/`）。
- 去重判定（Batch 2 跨域对齐时检查）：两个节点 `summary` 描述同一身份/同一状态机/同一真源，或主张中出现同一不变量 → 保留归属域那一个，另一个改成引用。

## 6. 证据规则

- 每条主张至少一个**符号引用**；做不到（配置文件、markdown 表、SQL 片段）才用行范围引用，并在同一节点写明基线提交。
- 优先级：符号引用（漂移可追踪）> 测试符号引用（证明有门守护，如 `[@ file="tests/session_data_sql_death_list.rs" symbol="fn …"]`）> 行范围引用 > 文件引用。纯文件引用只允许出现在上层/框架节点。
- 知识库语法没有 commit 属性：节点正文末尾写一行纯文本 `基线：<短哈希>`，表示证据在该提交下核对过。
- 引用必须在知识索引里为 `ok`；`symbol_drift` 视为证据失效，需重新核对后再提交。
- 结论可复现：任何人在基线提交上打开引用，能看到支撑主张的代码；不能依赖口头说明、聊天记录或本地临时文件。

## 7. 模板

### 7.1 Batch 2 节点（目录钉位：正文只一小段）

````markdown
```node
node : <key>
status : pending
domain : <域 id>
summary : <一句话：这个节点承诺什么>
```

**准入：** A<编号>。<一小段（≤4 句）：它凭什么是重要节点——守住了哪条不变量/状态机/真源/契约/提交边界/安全门；坏了会波及哪些模块。>

证据：[@ file="<path>" symbol="<chain>"]、[@ file="<test path>" symbol="fn <test>"]。
上层：[@ key="<parent>"]。
基线：<短哈希>
````

### 7.2 Batch 3 结论（二选一，追加在节点正文末尾）

结论 A —— 一致：

```markdown
**核实结论：一致，无漂移。**（基线 <短哈希>）
- <主张 1> —— [@ file="…" symbol="…"]；守护：[@ file="…" symbol="fn <test>"]（或：无测试守护，已记 ISSUE-xxx 为 疑似）。
- <主张 2> —— …
```

结论 B —— 无法拓展：

```markdown
**核实结论：无法拓展。** 主张与实现存在严重问题，见 ISSUE-<id>（<级别>）。（基线 <短哈希>）
- 承诺：<一句>
- 实际：<一句> —— [@ file="…" symbol="…"]
- 复现：<测试名或命令>
```

结论 B 的节点保持 `pending`，不得自行改为 `disabled`。

## 8. 写入纪律

1. `status : enabled` 的节点：未经 Andy 明确批准，不改一个字（含 summary、坐标、标签行）。需要修正时：记 issue（契约漂移）+ 在 pending 新节点里写正确版本，等 Andy 审。
2. `pending` 节点（含存量 9 个工具子契约）：只由归属域负责人修改；只做纠错和补证据，不重写主张；改动在 log 里留一条 `append_log`。
3. 优先新建一个带链接的节点，而不是重写已有节点。
4. 不改 `docs/knowledge-references.md` 与 `src/knowledge/*`；发现语法/索引问题记到 `knowledge` 域 issue。
5. 节点只在 `/workspace/litecode/knowledge/` 内创建；提交遵守提交纪律（攒量或高风险前才提交，独立分支）。

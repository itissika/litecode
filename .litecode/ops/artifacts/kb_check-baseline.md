# ops.kb_check 基线（存量知识库）

- 日期：2026-10-11（HKT）
- tools_version：7（ops.kb_check v1）
- 工作区 HEAD：`075308b`；分支 `ops/team-framework`
- 存量判定基线（stock_ref）：`e583ff2`（准入文档「基线提交」；该提交已存在的节点 = 存量）
- 复现：`node .litecode/ops/tools/kb_check.mjs`（JSON：`--json`）

## 可用性：cases.json 对照

`node .litecode/ops/tools/kb_check.mjs --selftest` → **87/87**（extract 30/30，resolve 56/56，layout 1/1），无不一致。

另外与 litecode 本体（`target/debug/litecode serve`，在隔离副本上调用 `/api/workspace/symbols` 与 `/api/workspace/knowledge`）做了逐文件对照：src/ 与 web/src 下 818 个 .rs/.ts/.tsx 文件以及另外 18 个 .mjs/.py 文件，作用域列表（符号链 + 行范围）全部一致；知识库解析结果（36 节点、205 条引用、0 个 litecode 级问题）一致。Go/C/C++/Java/C# 仓库中没有可对照的样本，按近似实现处理（找不到/不唯一 → `symbol_unverified` 警告）。

## 总数

| 项 | 数 |
|---|---|
| 节点 | 36（存量 36，新 0；enabled 26，pending 10） |
| 引用 | 205 |
| **error** | **0** |
| warning | 119 |

litecode 自身的状态码（missing_file / missing_symbol / dangling_relation / symbol_drift / inactive_target …）在当前 HEAD 上 **全部为 0**。

## 按 code 分布（全部是 warning，存量节点只提示）

| code | 数 | 说明 |
|---|---|---|
| `domain_missing` | 36 | 所有节点都没有 `domain : <域 id>` 行（准入 §1 新规，存量都不满足） |
| `baseline_missing` | 36 | 所有节点都没有 `基线：<短哈希>` 行（准入新规） |
| `dup_primary_suspect` | 32 | 16 对节点共用同一源码证据（每对两侧各报一次），全部在 `contracts/tools/*` 与 `tool-call-lifecycle` 一带 |
| `parent_missing` | 8 | 无 `上层：` / `父节点：` |
| `parent_multiple` | 5 | `上层：` 引用了 2 个节点 |
| `tag_unknown` | 2 | 不在五个标签内 |

## 典型样例

**parent_multiple（5）**：`engine-lifecycle`（contracts/engine-lifecycle.md:19）、`retrieval-contract`（:19）、`featured-tools`（framework/featured-tools.md:44）、`agent-harness-philosophy`（harness/agent-harness-philosophy.md:21）、`workspace-shared-substrate`（harness/workspace-shared-substrate.md:19）——均为 `上层：` 一行引用 2 个节点。

**parent_missing（8）**：`tool-control-boundary`、`tool-implementation-contract`、`tool-adapters`、`tool-authorization`、`tool-execution-context`、`tool-identity`、`tool-scheduling`、`why`（`why` 是根节点，缺上层属正常，后续可在工具里给根节点豁免）。

**tag_unknown（2）**：`why.md:15`「理念的落点：」；`design/engine-design.md:17`「关键契约：」。

**dup_primary_suspect（16 对）**：被多节点共用最多的证据——
- `sym:src/tool/executor.rs :: fn run_tool`（出现在 20 条重复提示里）
- `sym:src/tool/pipeline.rs :: impl ToolPipeline › fn execute_batch_cancellable`（6）
- `sym:src/tools/custom.rs :: impl Tool for CustomTool › fn execute`、`sym:src/tools/mcp_tool.rs :: impl Tool for McpTool › fn execute`、`impl McpTool › fn new`、`src/tool/registry.rs :: fn instantiate_tool`（各 2）

涉及的对：tool-adapters ↔ tool-call-lifecycle / tool-execution-context / tool-identity / tool-request-entry；tool-authorization ↔ tool-input-validation / tool-request-entry；tool-call-lifecycle ↔ tool-execution-context / tool-identity / tool-request-entry / tool-result-delivery / tool-scheduling；tool-execution-context ↔ tool-identity / tool-request-entry / tool-scheduling；tool-identity ↔ tool-request-entry / tool-result-delivery。
这是启发式（共享 symbol/range 证据，或 summary 完全相同），共用一个入口函数不一定就是重复主节点，需要人判断。

## 判断与差异记录

- `filename_mismatch` 在 litecode 是 warning，这里升为 error（准入 §0 写的是「必须」）。当前 0 条。
- 准入文档写 27 enabled / 9 pending，实际是 **26 enabled / 10 pending**（10 个 pending 全在 contracts/tools/）。
- 准入模板 §7.1 用 `证据：`，但标签清单写的是 `依据：`；两个都接受，`证据` 当作 `依据` 的别名。
- docs/knowledge-references.md 说 resolve 用例 54 条，cases.json 实际 56 条（以 cases.json 为准）。
- 存量节点不满足新规只报 warning，所以基线 error=0。准入新规要求的 domain/基线 两项在存量上是 36/36 全缺，如果要把存量也纳入硬约束，需要先回填。

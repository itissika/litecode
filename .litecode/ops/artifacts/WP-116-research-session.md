# WP-116-research-session · 三态口径重切（WP-119）

- 角色：Session；包 WP-119（父 WP-117）。只读；未改 src/tests；未 commit。时间 2026-10-13 HKT。
- 口径：稠密只留 **user + 助手消息 + 助手思考**；**不含任何 tool**（中层/GM 澄清）。
- 样本：`.tmp/wp102/corpus/sessions.db`，只读 `mode=ro&immutable=1`。29 会话、4336 行、跨度 2026-10-08～10-13（5 天）。标注：**清洗副本实测；跨度短，外推不代表长期**。
- 输出：`.tmp/wp116/out/three_state.json`（仅聚合）。脚本：`.tmp/wp116/three_state_stats.py`。Token：cl100k_base（同 `estimate.rs:56` / WP-104）。
- 裁定：Q1 非 reasoning 的 assistant → 助手消息；未知 kind →「未映射」；Q2 主+子分列；Q3 含空/不含空两行。

## B1 样本
见上。打开模式与规模见 json `meta`。

## B2 映射与 DISTINCT
锚点：`slots.rs:89-91`（Who/Said/Thought）、`corpus.rs:183-210`（Final 对照，含 tool）。

| kind | item_type | 条数 | 归入 |
|---|---|---:|---|
| item/user | message | 56 | 三态 · user |
| item/assistant | message | 149 | 三态 · 助手消息（Said） |
| item/assistant | reasoning | 727 | 三态 · 助手思考（Thought） |
| item/tool_call | function_call | 1228 | final 内 tool（不进三态） |
| item/tool_result | function_call_output | 1228 | final 内 tool（不进三态） |
| request/header | request/header | 794 | 未映射 |
| turn/start | turn/start | 47 | 未映射 |
| turn/end | turn/end | 42 | 未映射 |
| reminder/* | message | 65 | 未映射（五种 reminder 合计） |

本样本 assistant 非 reasoning **仅有 `message`**（与裁定一致）。无其它未知 assistant item_type。

## B3 对照表（cl100k token）

### 含空行

| 层 | 条数 | token | 主 token | 子 token |
|---|---:|---:|---:|---:|
| all | 4336 | 1,722,427 | 1,232,773 | 489,654 |
| final（现可嵌，含 tool） | 3388 | 1,582,667 | 1,115,271 | 467,396 |
| **three_state（Andy）** | **932** | **535,639** | **445,546** | **90,093** |

### 不含空行
本样本仅 1 条空行（子会话）；三态层无空行。不含空：all / final 各少 1 行，token 与含空相同；三态不变。可删占比与含空一致。

### 可删占比
| 相对 | 条数可删 % | token 可删 % |
|---|---:|---:|
| vs all | 78.51 | **68.90** |
| **vs final（给 IDE）** | **72.49** | **66.16** |

### 三态内构成（token）
- 助手思考 462,313（86.3%）；助手消息 69,448（13.0%）；用户 3,878（0.7%）。

### final 内 tool 拆分（稀疏/稠密根因输入）
| | 条数 | token | 占 final token % |
|---|---:|---:|---:|
| tool_call | 1228 | 87,713 | **5.54%** |
| tool_result | 1228 | 959,315 | **60.61%** |
| tool 合计 | 2456 | 1,047,028 | **66.16%**（与「三态相对 final 可删」一致：final − 三态 ≈ 全部是 tool） |

**给 IDE 一句：** 按 Andy 三态砍掉后，相对现 final 可删约 **66% token（72% 条数）**；其中 final 超额几乎全是 tool，且 **tool_result 约占 final 的 61%**，tool_call 约 5.5%。

## B4 局限
- 跨度仅 5 天；`token_estimate` 列合计 1,624,851，与 cl100k all 偏差约 −5.7%（以重算为准）。
- 嵌入模型 tokenizer ≠ cl100k → IDE。
- 未映射（request/header、turn、reminder）只在 all 层，不在 final；砍三态相对 final 时它们不进入「可删」差额。

## 隐私
产物与 json 无正文、无会话 id 列表。

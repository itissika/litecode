# WP-116-research · 语义检索续测：语料本质与稠密/稀疏对齐（WP-117 方案）

- 角色：backend_mid；父 WP-116（gm，approver=andy）；本包 WP-117（in_progress）。
- 规则：**只读续测**；不改 `src/`、`tests/`；脚本仅 `.tmp/wp116/`；不 commit；不碰 `windows-*.yml`。
- 时间：2026-10-13 21:0x HKT。
- 继承：WP-101 已封（LEAN/护栏/续嵌建议）；本包**不开落地实现**，为全面落地包防走弯路。

## 1. Andy 锁定口径
1. **继续测，不落地。**
2. **稠密语料只留三态**：`user` 消息、助手消息、助手思考（reasoning）；**其他全删**（含 `tool_call` / `tool_result`、control、reminder、echo 等）。
3. 稀疏腿相对对话**只应多 `tool_call`（动作）**；**`tool_result` 不算「该多的」**。稠密三态**不含任何 tool**。**稠密没有理由比稀疏语料还多**——查清现状为何偏大，列出本不该有的开销与逻辑。
4. 关注序：①产品本质砍冗余 → ②性能 → ③检索质量。
5. **GM 收口（2026-10-13）**：A1 差因表必须按「该有 / 不该有」标注——稀疏该有=对话三态 + tool_call；不该有=tool_result 及更杂项进稀疏或任何 tool 进稠密。

## 2. 子包拆分
| 包 | owner | 范围 | 产物 |
|---|---|---|---|
| WP-118 | ide | 主力：稠密 vs 稀疏路径只读对照；按三态口径过滤后体积/chunk/嵌入对照；「不该进稠密」逻辑清单；质量假说 | `WP-116-research-ide.md` |
| WP-119 | session | 小包：按三态口径重切 `sessions.db` 只读统计，给 IDE 对照 | `WP-116-research-session.md` |

顺序：WP-118 ∥ WP-119；Session 数字经中层转 IDE 用于「过滤后省多少」。中层合并进本文件 §6 后 deliver WP-117。

## 3. 每包问题与验收

### A · WP-118 IDE（engines / session_search）
**A1 现状差因（产品本质）**  
只读核对 `Policy::Final` / slots / trimmer / dense `semantic_index` / sparse lane：
- 稠密当前实际纳入哪些 kind / slot（含 tool？echo？control？）
- 稀疏 Final 纳入哪些；相对「纯对话」多了什么（**该有仅 `tool_call`；`tool_result` 标不该有**）
- **稠密体积/条数/token/chunk 是否 ≥ 稀疏**；若是，逐项列出「本不该进稠密」的逻辑与代码锚点（文件:行）
- 输出：**可删开销/逻辑清单**（每条：现状 → **该有/不该有** → 删后预期）

**A2 三态口径过滤对照（测量）**  
在**不改 src** 前提下，用 `.tmp/wp116/` 脚本或现有只读 API/导出，对同一语料算：
| 口径 | 条数 | token（注明 tokenizer） | chunk 数 | 相对现状 Δ |
|---|---:|---:|---:|---:|
| 现状稠密（Final 等） | | | | 0 |
| Andy 三态（user+助手消息+思考） | | | | |
| 稀疏 Final（现状） | | | | |
| 稀疏「对话三态+仅 tool_call」（该有口径；若可只读近似） | | | | |
| 稀疏含 tool_result 的现状超额（不该有） | | | | |

嵌入耗时：可用 WP-102 吞吐外推或护栏下小样本；**强制** `.tmp/wp102/run_limited.sh` 级护栏若跑真模型；默认 LEAN；**不测 batch=8**。

**A3 性能（次于本质）**  
在「已按三态砍」假设下：剩余嵌入量、LEAN 收益是否仍关键；指出哪些 WP-101 建议（续嵌/LEAN）在新口径下仍成立或降级。不实现。

**A4 检索质量假说（再次之）**  
不改代码。基于现有 relevance/golden/手测 query（或 WP-104 语料抽问）：
- 删 tool_result 后：工具失败排查类查询的风险
- 稠密只留三态后：相对稀疏+工具调用的分工是否更清晰
- 列出 **收益假说 / 风险假说 / 需落地包验证的用例**（不做评测实现）

验收：A1 清单非空且有代码锚点；A2 表有方法+样本范围；A3/A4 有明确「做/缓做」建议。

### B · WP-119 Session
**B1** 只读打开既有样本（优先 `.tmp/wp102/corpus/sessions.db` 或 Andy 副本；注明路径与跨度）。  
**B2** 按三态口径重切：保留 kind/item 映射需写清（user / assistant text / assistant reasoning；其余丢弃）。  
**B3** 对照表：all / 现 final / 三态 的条数+token；三态相对 final 可删占比。  
**B4** 标注样本局限（天数等）；产出聚合 json（无正文）。

验收：表+方法+给 IDE 的「可平移百分比」一句。

## 4. 证据要求
同 WP-101：实测附方法/环境/样本；公开附链接+日期；区分实测/公开/推断。脚本仅 `.tmp/wp116/`。

## 5. 发现（子包回填，中层合并）
> 合并 2026-10-13 ~21:15 HKT。WP-119 done；WP-118 review 通过。口径：稠密=三态无 tool；稀疏该有=三态+仅 tool_call；tool_result 双侧不该有。

### 5.1 稠密 vs 稀疏差因与可删清单（A1）【实测·代码】
**根因**：`Policy::Final`（`corpus.rs:188-194`）把 Who/Said/Thought/**Did/Outcome** 一并写入稠密；稀疏 `derive.rs:96` 也不挡 tool_result。双侧同吃 Outcome → 稠密并不更瘦，且按 chunk 嵌入体感更重。

| 项 | 稠密 | 稀疏 | 该有/不该有 | 锚点 |
|---|---|---|---|---|
| 对话三态 | 进 | 进 | **该有** | `slots.rs:89-91` |
| tool_call | 进 Did | 进 | 稠密**不该有**；稀疏**该有** | `slots.rs:92`; `corpus.rs:192` |
| tool_result | 进 Outcome | 进 | **双侧不该有** | `slots.rs:93`; `derive.rs:96` |
| reminder/control | Final 多半不进 | derive 可能进 | 稀疏**不该有** | `slots.rs:157-164` |
| echo tool_result | 已剔 | 已剔 | 正确（保留） | `corpus.rs:90-94` |

可删优先序：①稠密去 Outcome ②稠密去 Did ③稀疏去 tool_result ④稀疏收敛 reminder/control。

### 5.2 三态过滤对照（A2+B）
样本同 `.tmp/wp102/corpus/sessions.db`（5 天，外推不可靠）。

| 口径 | 条数 | token | 备注 |
|---|---:|---:|---|
| all | 4336 | 1.72M cl100k | WP-119 |
| final（现状可嵌） | 3388 | 1.58M cl100k / **1.55M granite** | 含 tool |
| **Andy 三态** | **932** | **536k cl100k / 501k granite** | −66～−68% vs final |
| 稀疏该有（三态+tool_call） | 2160 | **590k granite** | 该有口径 |
| tool_result 超额 | 1228 | **955k granite（~62% of dense）** | **不该有·主因** |

砍到口径后：**稠密 0.50M < 稀疏该有 0.59M**，关系恢复。嵌入外推：现状 ~10 min → 三态 ~3 min（未再跑真模型）。  
给落地：相对 final 可删约 **66% token**；几乎全是 tool，其中 result≈61%、call≈5.5%。

### 5.3 性能含义（A3）
- **第一刀=改准入**（砍语料）＞ WP-101 清洗 ~14%。  
- LEAN/续嵌仍成立但次于口径；换模/云仍不做。

### 5.4 质量假说（A4）
- 收益：稠密专对话语义；稀疏专动作（tool_call）。  
- 风险：失败输出原文排查变弱；「跑过 X」靠稀疏兜。  
- 落地包验证：失败 cargo/pytest 输出；read/edit 路径；纯设计讨论；短 user 句（用户仅 ~0.7% token）。

### 5.5 结论
1. 稠密偏大的产品根因是 Final 把 tool（尤其 Outcome）当对话一起嵌，不是权重或 LEAN。  
2. 按 Andy 口径砍后嵌入量约 −2/3，且稠密重新小于「稀疏该有」。  
3. 性能旋钮是第二优先级；质量需用例验证删 result 的代价。

### 5.6 推荐组合（给未来落地包）
1. 稠密 Final：**只保留 Who/Said/Thought**。  
2. 稀疏：保留 tool_call；**去掉 tool_result**；收敛 reminder/control。  
3. 保留 echo 剔除；并行续嵌；LEAN 作压测/默认护栏。  
4. 落地包带 §5.4 四类用例。

### 5.7 不做清单
- 本续测包不改 src。  
- 不靠「清洗 14%」替代砍 tool_result。  
- 不为省 RAM 换小模型/上云。  
- 不新增 Warming/状态机。

### 5.9 compact 剔除 vs 检索准入（Andy 追问 · 2026-10-13）
对照 compact 管线与稠密/稀疏 Final 准入。**Compact 不删历史 detail**（`session.rs:2921-2923`：工作集=当前 summary + `seq>=kept_from`；检索仍按 `INDEXED_KINDS` 扫库）。

| 内容 | Compact 怎么剔 | 稠密 Final | 稀疏 derive | 归类 |
|---|---|---|---|---|
| **tool_result 长正文** | 进摘要前截到 **2000 字**（`keep_recent.rs:13,174-198`）；工作集折叠后模型看不见旧段，但行仍在库 | **仍进** Outcome（`corpus.rs:188-194`；投影预算裁，`slots.rs:186`） | **仍进**（`derive.rs:96`；chunk 侧 Outcome 再裁 **400 字**，`trimmer.rs:20-41`） | **compact 已视为低价值；检索仍全量/半量进** → 优先对齐（至少稠密不进；稀疏若留也不该高于 compact 摘要预算） |
| **媒体（图/文件 part）** | 摘要：**全部 strip**（`media_budget::strip_media_for_summary`）；模型视图：**看过即丢** `drop_stale_media` | 若落成可抽正文/blob 文本仍可能被 `row_plain_text` 抽到 | 同左 | **两边都不该进检索**（索引文本通道无媒体语义；对齐 compact） |
| **compact 摘要里的 reasoning** | 写 checkpoint 时**只留 assistant Output**（`compact.rs:499-507`） | Thought 槽：**Andy 三态要留**（原文思考，非摘要里的 think） | 可进 | **勿混**：compact 不把 think 写入摘要 ≠ 检索不该索引历史 Thought；三态口径仍留助手思考 |
| **compacted 行本身** | 工作集可见摘要 | Final：**slot Summary → 不进**（`corpus.rs:188-194`） | **`kind==compacted` 直接拒**（`derive.rs:96`） | **已对齐（检索不该进）** |
| **被折叠掉的旧 detail（含三态+tool）** | 模型工作集不再展示 | **`searchable_keys` 仍索引**（`read.rs:451,482`：`state!='in_progress'` + 四 kind；**不看 kept_from**） | 同左 | **compact 已剔出模型上下文；检索仍进** → 产品裁定：检索跟「全历史」还是跟「工作集视图」 |
| **echo tool_result** | （非 compact 规则） | 已剔 | 已剔 | **已对齐** |
| **reminder / control** | compact 无专门剔；亦不在 INDEXED_KINDS | Final 多半不进 | derive 可能进（若被写成 searchable kind 则不在四 kind 外） | INDEXED_KINDS 已不含 reminder；与 WP-118 稀疏「8 kinds」注释需落地时再钉 |

**给落地范围的建议（供裁定，非实现）**  
1. 与 §5.6 一致：稠密去 Outcome/Did；稀疏去 tool_result（或至多按 ≤2000 字对齐 compact 摘要预算，而不是 400/512 各一套）。  
2. 明确媒体：检索侧保证不索引 media part（对齐 strip）。  
3. **历史折叠**：是否让 `searchable_keys` 尊重 `kept_from`（只索引工作集可见行 + compacted 摘要投影）——这是比「三态」更大的产品刀，建议单独裁定。  
4. 不要因 compact 摘要丢 reasoning 而从稠密删 Thought。

### 5.8 待 Andy/GM 裁定
- 是否立即开**落地实现包**按 §5.6 改准入（稠密三态 / 稀疏去 Outcome）。  
- 删 tool_result 后，失败输出是否接受「回会话原文」而不另建索引通道。  
- 稀疏 reminder/control 是否同包收敛还是随后。  
- **§5.9**：检索是否对齐 compact——(a) tool_result 不进或 ≤2k；(b) 媒体永不进；(c) searchable 是否跟 `kept_from` 工作集视图（更大刀）。

## 6. 合并计划
子包 deliver → 中层审锚点与数字 → 填 §5 → WP-117 deliver → GM 封 WP-116。

## 7. 不在本包
任何 src/tests 改动；换模型；云嵌入；UI；落地续嵌/LEAN 默认（可引用 WP-101，不实现）。

## 8. 验收（WP-117）
- [x] WP-118/119 done 或明确 blocked
- [x] §5 有带锚点的可删清单 + 对照表 + 推荐/不做
- [x] 未改 src/tests；validate_all ok；未 commit

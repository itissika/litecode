# WP-120-research-ide · 白名单 · 切块 · 算法小样本（WP-122）

- owner：ide；父 WP-121 / WP-120；包 WP-122。
- 规则：只读；不改 `src/`/`tests/`；脚本/CSV 仅 `.tmp/wp122/`；**不写** `.tmp/wp120/out/`（Session 正式集只读）。
- 时间：2026-10-13 21:2x HKT。
- 正式集（RO）：`.tmp/wp120/out/queries.json`（seed=123，n=10，failure_triage×2 `expect_degraded`）、`whitelist_slices.json`。
- 证据标【实测/推断】。

---

## 口径草案（白名单 · 只要什么）

| 腿 | **只要** |
|---|---|
| **稠密** | `item/user`；`item/assistant`（消息）；`item/assistant`+`reasoning`（思考） |
| **稀疏** | 上列三态 + `item/tool_call` |

**产品话术**：「search 只取 {稠密三态；稀疏=三态+tool_call}；session 新增 kind 默认不可搜，除非改白名单。」  
（与 Session `product_line` 一致。）

---

## A · 白名单 vs 现状

### 代码缺口【实测】
| | 现状 | 白名单要 | 锚点 |
|---|---|---|---|
| 稠密 | Final 准入 Who\|Said\|Thought\|**Did\|Outcome** | 仅三态（无 Did/Outcome） | `corpus.rs:188-194`；`slots.rs:176-187`；`semantic_index.rs:530-533` |
| 稀疏 | `derive_row`：有文且非 compacted/echo（可含 tool_result 等） | 三态+tool_call | `derive.rs:89-96` |

### 体积【实测】样本 `.tmp/wp102/corpus/sessions.db`；granite=`tokenizer.json`；chunk≈ceil(tok/448)

| 口径 | 条数 | cl100k | granite | chunks≈448 |
|---|---:|---:|---:|---:|
| 现状稠密（Final 五槽） | 3387 | 1,568,588 | **1,545,357** | 5889 |
| **稠密白名单三态** | 932 | 521,560 | **500,851** | 1723 |
| **稀疏白名单（三态+tool_call）** | 2160 | 609,273 | **590,224** | 2995 |
| tool_result（不在白名单） | 1227 | 959,315 | 955,133 | 2894 |

稠密白名单相对现状：**−67.6% granite**。Session 切片 cl100k 见 `whitelist_slices.json`（deltas 同量级）。原始：`.tmp/wp122/out/whitelist_vol.json`。

---

## B · 切块小样本

语料：稠密白名单 932 行。脚本硬切近似（+多块时 1 条 head+tail anchor）。外推 WP-103 ≈10 c/s / LEAN 7.8 c/s。【实测·外推】未再跑真模型。

| 档 | chunks | 墙钟@10c/s | @7.8 LEAN |
|---|---:|---:|---:|
| **448+anchor（现状）** | 2042 | 204 s | 262 s |
| 256+anchor | 2953 | 295 s | 379 s |
| 512 无 anchor | 1602 | 160 s | 205 s |

CSV：`.tmp/wp122/out/chunk_matrix.csv`。  
召回列：见 §C（同正式 query）；切块档未改检索实现，本包不声称「某档召回更好」。**倾向**：先钉白名单；切块默认维持 448，512 可作省嵌入备选（待落地包+真引擎验证）。

---

## C · 算法小样本（硬门槛）

### 方法【实测·offline 近似】
- Query：正式 `queries.json` **只读**（n=10）。
- 索引：白名单稀疏 / 白名单稠密；诊断对照 full5（含 tool_result）。
- 检索：BM25（CJK bigram）+ hash128 余弦；融合 `RRF_K=60`、`w_s=w_d=1`（对齐 `mod.rs:280-284` / `ranking.rs:372-376`）。
- 对照：sparse-only；`w_d=0.5`；full5 稀疏诊断。
- **非**生产 serve/ANN 路径。

表：`.tmp/wp122/out/algo_matrix.csv`、`algo_summary.json`。

### 主指标：exact-seq hit@10

| intent (n) | sparse | dense | RRF | sparse-only | RRF w_d=0.5 | full5 诊断 |
|---|---:|---:|---:|---:|---:|---:|
| short_user (2) | 0 | 0 | 0 | 0 | 0 | 0 |
| pure_dialogue (2) | 0 | 0 | 0 | 0 | 0 | 0 |
| assistant_thinking (1) | 0 | 0 | 0 | 0 | 0 | 0 |
| tool_intent (3) | 0 | 0 | 0 | 0 | 0 | 0 |
| failure_triage (2, degraded) | 0 | 0 | 0 | 0 | 0 | 0 |
| **合计** | **0/10** | **0/10** | **0/10** | **0/10** | **0/10** | **0/10** |

原因【推断+抽查】：WP-123 gold `seq` 按长度种子挑选，与 query 词面常无交集（例 q04 `subagent_send` ∉ gold tool_call 正文）；offline 词法/hash 无法当作相关性评测。

### 次指标：kind-bucket 是否出现在 top-10（结构性，非相关性）
- 对话/短 user / 部分 tool_intent：稀疏或 RRF 常能顶到对应桶（桶命中偏高）。
- **failure_triage**：白名单稀疏/稠密/RRF 的 tool_result 桶命中 **0**；full5（含 tool_result）q06 桶命中 1——与 `expect_degraded: true` 方向一致【实测】。

### 硬门槛结论
**有小样本表，但不得写产品算法推荐。**  
可写：「不足以下结论，待真引擎+相关性金标加测；白名单使 failure_triage（gold∈tool_result）在稀疏/稠密上变弱，符合预期。」  
**禁止**：推荐上线某 RRF 权重 / 关 dense / 换融合。

---

## 推荐组合 / 不做 / 待裁定

**推荐（有体积+代码证据，非算法调参）**  
1. 落地白名单准入（稠密三态；稀疏三态+tool_call）。  
2. 切块默认仍 448；512 仅作成本备选待验。  
3. 算法：保持现状 RRF 实现作基线，**本包不改权重建议**。

**不做**  
- 本包不改 src。  
- 不依据本 offline 表上线算法变更。  
- 不为省内存换模/上云。

**待裁定 / 加测**  
- 真引擎路径上对正式 query 重跑 hit@k（或换相关性标注金标）。  
- kept_from / UI 不在本包。

## 原始数据（仅 wp122）
- `.tmp/wp122/out/whitelist_vol.json`
- `.tmp/wp122/out/chunk_matrix.csv`
- `.tmp/wp122/out/algo_matrix.csv` / `algo_summary.json`
- （历史）`queries_ide.json` 已弃用，评测以 wp120 正式集为准

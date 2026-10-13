# WP-116-research-ide · 稠密/稀疏语料差因与三态口径（WP-118）

- owner：ide；父 WP-117 / WP-116；包 WP-118。
- 规则：只读；不改 `src/`、`tests/`；脚本 `.tmp/wp116/`；不 commit。
- 时间：2026-10-13 21:1x HKT。
- 样本：`.tmp/wp102/corpus/sessions.db`（与 WP-119 同库；29 会话 / 5 天 2026-10-08～13）。
- 证据：【实测】【公开】【推断】。

## Andy / GM 口径（验收尺子）
| | 该有 | 不该有 |
|---|---|---|
| **稠密** | user、助手消息、助手思考 | **任何 tool**（call+result）、control、reminder、echo… |
| **稀疏** | 对话三态 + **仅 tool_call** | **tool_result** 及更杂项 |

---

## A1 · 差因与可删清单（产品本质）

### 1.1 现状：谁进了哪条腿

**稠密（`Policy::Final`）**【实测·代码】  
- 准入：`corpus.rs:188-194` — slot ∈ `{Who,Said,Thought,Did,Outcome}`（注释明确「人话 + 工具调用 + 工具产出」）。  
- 投影：`slots.rs:176-187` `row_text_final` — Who/Said/Thought 按预算截断；**Did（tool_call）保留且预算 ×1.5**；**Outcome（tool_result）原样截断进语料**。  
- 建索：`semantic_index.rs:530-549` 走 `Policy::Final` + echo 剔除。  
- kind 白名单：`slots.rs:159-167` / `corpus.rs:181-184` 含 `tool_call`/`tool_result`（另有 reminder 等在 `kind_included`，但 Final 的 slot 匹配会丢掉 Other/Summary/When）。

**稀疏**【实测·代码】  
- 派生：`derive.rs:89-96` — 有文本、非 `compacted`、非 echo → **admit**；**不走 `Policy::Final` slot 过滤**。  
- 因此：凡 searchable 行带正文的 tool_call / tool_result（非 echo）都会进 FTS；若上游仍投递 reminder/control，也会进（`slots.rs:157-158` 自注「FTS 8 kinds vs dense 5」不一致）。

**Echo**【实测】`corpus.rs:88-94` / `echo.rs`：session 回声的 **tool_result 整条剔除**，只留对应 tool_call — 双侧一致；**不能**解释「稠密偏大」。

### 1.2 差因表（该有 / 不该有）

| 项 | 稠密现状 | 稀疏现状 | **该有/不该有** | 锚点 | 删后预期 |
|---|---|---|---|---|---|
| user / 助手消息 / 助手思考 | 进（Who/Said/Thought） | 进 | **该有**（双侧对话） | `slots.rs:89-91`, `corpus.rs:192` | — |
| tool_call | 进（Did） | 进 | 稠密 **不该有**；稀疏 **该有** | `slots.rs:92,183-186`; `corpus.rs:192` | 稠密 token/chunk↓；稀疏保留动作检索 |
| tool_result | 进（Outcome，仅非 echo） | 进（非 echo） | **双侧不该有** | `slots.rs:93,186`; `corpus.rs:192-193`; `derive.rs:96` | **主因**：本样本占稠密 granite **~62%**；砍后稠密≪现状稀疏 |
| compacted / turn / Summary/When | Final 丢 | compacted 丢；turn 视上游 | **不该有** | `corpus.rs:179`; `derive.rs:96` | 已基本不进稠密 |
| reminder / control / plan | Final slot→Other→不进稠密 | derive 可能进 | 稀疏侧 **不该有**（超出口径） | `slots.rs:55,163-164`; `derive.rs:96` | 稀疏向「三态+tool_call」收敛 |
| echo tool_result | 已剔除 | 已剔除 | 正确 | `corpus.rs:90-94` | — |

### 1.3 为何「稠密没有理由比稀疏还多」却显得偏大
1. **口径错位**【推断·对齐实测】：Andy 期望稠密⊂稀疏（稀疏只多 tool_call）。**现状稠密与稀疏在 tool 上同吃 tool_result**，稠密并不更「瘦」。  
2. **体积主因**【实测】：`tool_result` ≈ dense_now granite 的 **61.8%**（955k/1.55M）；再切 448-token → **~2894 chunk**，嵌入墙钟被它绑架。  
3. **同文不同价**【推断】：稀疏把大段 outcome 进 FTS 一次；稠密按 chunk **逐条 ORT**，体感「稠密更重」。  
4. 相对「稀疏该有口径」（三态+tool_call）：现状稠密 granite **+162%**（见 A2）——完全由不该有的 tool（尤其 result）造成。

### 1.4 可删开销清单
| # | 现状 | 判定 | 删后预期 |
|---|---|---|---|
| 1 | Final 把 `Slot::Outcome` 写入稠密 | **不该有** | 稠密 granite ≈ −62%；chunk ≈ −2894 |
| 2 | Final 把 `Slot::Did` 写入稠密 | **不该有** | 再 −~89k granite / ~1272 chunk；动作检索交给稀疏 |
| 3 | 稀疏 `derive_row` 仍索引 tool_result | **不该有** | 稀疏向该有口径收敛；字面证据回会话原文 |
| 4 | 稀疏可能索引 reminder/control（8 kinds） | **不该有** | 与对话+tool_call 分工清晰 |
| 5 | （保留）echo 剔除 | 该有逻辑 | 勿回退 |

---

## A2 · 三态过滤对照（测量）

**方法**【实测】：`.tmp/wp116/out/corpus_compare.json` — 只读 sqlite `state=final` 行，正文抽取启发式（与 WP-119 略差，见下）；token = 引擎 `tokenizer.json`（granite）+ 对照 cl100k；chunk ≈ `ceil(granite_tokens/448)`（`DENSE_CHUNK_TOKENS`）。  
WP-119 官方 cl100k：三态 535,639 / final 1,582,667（`WP-116-research-session.md`）；本侧三态 cl100k **521,560**（抽取差 ~2.6%，不改结论）。

| 口径 | 条数 | cl100k | **granite** | chunk≈ | 相对现状稠密 Δ granite |
|---|---:|---:|---:|---:|---:|
| 现状稠密（Final 五槽） | 3387 | 1,568,588 | **1,545,357** | 5889 | 0 |
| Andy 三态 | 932 | 521,560 | **500,851** | 1723 | **−67.6%** |
| 稀疏 Final 近近似（同五槽） | ≈3387 | ≈1.57M | ≈1.55M | （FTS 按行） | ≈0（同源 tool） |
| 稀疏「三态+仅 tool_call」（该有） | 2160 | 609,273 | **590,224** | 2995 | （对照） |
| 稀疏 tool_result 超额（不该有） | 1227 | 959,315 | **955,133** | 2894 | — |

要点：
- 三态相对现状稠密：**−67.6% granite / 条数 932 vs 3387**（与 WP-119「可删 ~66% token」一致量级）。  
- 砍到口径后：**稠密 501k < 稀疏该有 590k** → 「稀疏只多 tool_call」关系恢复。  
- 嵌入外推【推断×WP-103 吞吐】：现状 ~5889 chunk ÷ 10 c/s ≈ **9.8 min**；三态 1723 ÷ 10 ≈ **2.9 min**（LEAN≈3.7 min）。**未再跑真模型**（外推足够；护栏脚本已备 `.tmp/wp116/run_limited.sh`）。

---

## A3 · 性能（次于本质）

| WP-101 建议 | 新口径下 |
|---|---|
| 砍语料冗余 | **升为第一刀**：单砍 Outcome+Did 出稠密即可 −67% 嵌入量，大于清洗 ~14% |
| ORT LEAN / arena | 仍有用（盒内存），但优先级下降：语料砍完峰值压力正比下降 |
| 续嵌批提交 | **仍成立**：积压变短仍怕中断丢进度 |
| 换小模型 / 云嵌入 | 仍 **不做**（与 WP-103 一致） |

**做**：落地包先改 Final 准入（稠密三态；稀疏去掉 Outcome）。**缓做**：LEAN 调参、续嵌可并行但不阻塞口径。

---

## A4 · 检索质量假说（再次之）

**收益假说**  
- 稠密专注对话语义（含思考），不再被工具日志噪声拉偏。  
- 稀疏专责「做了什么」（tool_call 路径/命令），字面检索更干净。

**风险假说**  
- 删 tool_result：依赖「报错原文 / 大段命令输出」的排查类查询变弱 → 需会话打开原文或别的工具通道。  
- 删稠密 tool_call：语义「我跑过 X 命令」类跨会话回忆变弱 → 稀疏应能兜住标识符。

**待落地包验证用例**（不实现评测）  
1. 查某次失败的 `cargo`/`pytest` 输出片段（现靠 outcome）。  
2. 查某文件路径是否曾被 `read`/`edit`（靠 tool_call 参数）。  
3. 纯产品讨论 / 设计结论（应几乎只打稠密三态）。  
4. 中英混合短 user 句（三态用户仅 ~0.7% token，注意召回）。

---

## 推荐 / 不做（供中层 §5 合并）

**推荐**  
1. 稠密 Final：只保留 Who/Said/Thought。  
2. 稀疏：保留 tool_call；**去掉 tool_result**；收敛 reminder/control。  
3. 续嵌仍做；LEAN 作护栏默认。

**不做（本阶段）**  
- 本包不改 src。  
- 不为省 RAM 换模/上云。  
- 不靠「再清洗 14%」替代砍 tool_result。

## 原始数据
- `.tmp/wp116/out/corpus_compare.json`
- `.tmp/wp116/out/three_state.json`（WP-119）
- `.tmp/wp116/plan.md`、`run_limited.sh`

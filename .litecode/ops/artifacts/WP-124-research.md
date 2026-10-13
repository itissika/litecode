# WP-124-research · 真引擎算法与切块定口径（WP-125 方案）

- 角色：backend_mid；父 WP-124（gm，approver=andy）；本包 WP-125（in_progress）。
- 规则：**只读，仍不落地**；不改 `src/`、`tests/`；脚本仅 `.tmp/wp124/`；不 commit；不碰 `windows-*.yml`。
- 时间：2026-10-13 21:4x HKT。
- 继承：WP-120 已封——白名单钉死（稠密三态；稀疏三态+`tool_call`）。本包补 **生产路径真引擎** + **可对齐金标**（WP-122 offline/length-seed 不足）。

## 1. Andy / GM 锁定口径
1. 仍只读不落地。
2. 测 **生产路径**：真 ANN + RRF/排序（非仅 BM25/hash offline 近似）。
3. **金标必须与 query 可对齐**：丢掉 length-seed 那种对不上的抽法；gold seq 的正文词面应能支撑短 query。
4. 语料：**白名单** 上测 sparse / dense / 融合；切块 ≥2 档（**含 448**）。
5. **硬门槛**：无小样本数字 → 不得写算法产品推荐。
6. 护栏：`run_limited`；LEAN；**不测 batch=8**。

## 2. 子包
| 包 | owner | 范围 | 产物 |
|---|---|---|---|
| WP-126 | session | 可对齐金标 query 集（白名单语料；无长正文） | `WP-124-research-session.md` |
| WP-127 | ide | 真引擎 ANN+RRF + 切块小样本表；口径/不足声明 | `WP-124-research-ide.md` |

顺序：WP-126 尽早给正式 `queries.json`；WP-127 可先摸底生产路径，**正式表必须以 WP-126 金标为准**（禁止覆盖 `.tmp/wp124/out/` Session 文件；IDE 输出只写 `.tmp/wp124/ide/`）。

## 3. 验收细则

### A · Session（WP-126）可对齐金标
- 样本：复用 `.tmp/wp102/corpus/sessions.db`（ro）；白名单映射同 WP-120/123。
- ≥8 条（建议 ≥10）；覆盖：短 user / 纯对话 / 助手思考 / 工具意图 / 失败排查（`expect_degraded: true`，gold 可含 `tool_result` 但不计入白名单体积）。
- **对齐规则（硬）：** 从候选 row 的正文抽/改写短 query（≤80 字），使 query 词面与该 gold 行有可核验交集（脚本可记 `overlap_tokens` 或抽查表）；**禁止**纯 length-seed 随机 seq。
- 字段：`id`、`query`、`gold[{session_key=sha256[:12], seq, kind_bucket}]`、`intent`、`expect_degraded`；无长正文、无明文 session_id、无绝对路径。
- 数据：`.tmp/wp124/out/queries.json`（+可选 `whitelist_slices.json` 复用/重算）。
- 产物写清复现命令与对齐抽查（≥3 条示例：query ↔ gold 关键词，**不落整段 body**）。

### B · IDE（WP-127）真引擎 + 切块
**B1 生产路径摸底【实测】**  
锚点：ANN / dense index / sparse 多层 / `ranking.rs` RRF_K=60 / 融合入口（补精确文件:行）。说明如何只读驱动（现有 serve/CLI/脚本），不得改 src。

**B2 算法小样本（硬门槛）**  
白名单语料索引上，对 WP-126 正式 query：
| 模式 | 要求 |
|---|---|
| sparse-only | 生产稀疏路径 |
| dense-only | 真 ANN/稠密 |
| 融合现状 | 生产 RRF（默认权重） |
| 对照 ≥1 | 如关 dense / 改权重脚本侧模拟 / 单层 |

每条报 hit@5 / hit@10（exact-seq；可附 kind-bucket 次指标）。汇总按 intent；failure_triage 单独看 expect_degraded。  
输出：`.tmp/wp124/ide/algo_matrix.csv` + 方法命令。  
无表 / 不足 → **只写「不足以下结论，待加测」**；禁止推荐上线某算法/权重。

**B3 切块 ≥2 档（含 448）**  
同白名单语料：现状 448+anchor vs ≥1 备选（256 或 512）；报 chunk 数、估/测墙钟（护栏 LEAN；外推不够才 limited 真嵌入）。召回列绑同一金标（若切块档需重建索引则注明成本）。

**B4 口径草案素材**  
白名单表述继承 WP-120；本包只补「算法/切块：可裁定 or 明确不足」。

## 4. 证据与护栏
- 标实测/推断；样本 5 天局限。
- `.tmp/wp124/run_limited.sh`；LEAN；拒 batch=8。
- 不改 src/tests；validate_all ok。
- Session 正式集只写 `.tmp/wp124/out/`；IDE **不得覆盖**该目录。

## 5. 发现（回填）
来源：WP-126 done + WP-127 done（2026-10-13）。样本 5 天；n=10 小样本。

### 5.1 可对齐金标与方法
- 正式集：`.tmp/wp124/out/queries.json`（seed=126，n=10；五类各 2）。
- 方法：内容哈希选行 → 词面拼短问 → `overlap_tokens` 核验（一般 ≥2；short_user ≥1）；**已废除 length-seed**。
- failure_triage×2：`expect_degraded: true`，gold 含 `tool_result`。
- 缺口：自动拼问可读性一般；词面交集≠语义金标。

### 5.2 真引擎算法小样本【实测·生产 granite+usearch】
exact-seq hit@10（正式金标）：

| 模式 | hit@10 | hit@5 |
|---|---:|---:|
| sparse-only | 2/10 | 2/10 |
| dense-only（三态） | 2/10 | 1/10 |
| **RRF 融合（现状）** | **4/10** | 2/10 |
| dense_wl_residual（含 Did 诊断） | 2/10 | 1/10 |

- failure_triage 各模式 0（符合 expect_degraded）。
- 相对 WP-122 offline 0/10：生产路径给出非零命中。
- RRF 本金标上高于单腿；**不足以下产品结论**（n=10、未扫权重）→ **不写算法产品推荐**。

### 5.3 切块对照
| 档 | chunks | 墙钟 | 召回 |
|---|---:|---|---|
| **448+anchor（真索引）** | **1044** | **187 s LEAN** | 绑 §5.2 |
| 256+anchor | 估 2953 | 外推 | N/A（需改 src） |
| 512 无 anchor | 估 1602 | 外推 | N/A |

倾向（非落地裁定）：维持 448；512 作省嵌入候选待落地包。

### 5.4 口径草案 / 不足声明
- 白名单继承 WP-120（稠密三态；稀疏+tool_call）。
- 算法：**有表、可相对比较；禁止据此改线上权重/下线某腿。**
- 切块：默认 448 有实测；备选无召回列。

### 5.5 推荐组合 / 不做 / 待裁定
- **可讨论（非本包落地）：** 白名单准入（体积证据见 WP-120）；切块默认 448。
- **不做：** 改 src；依本表上线算法变更；换模/上云；对 256/512 声称召回优劣。
- **待加测：** 更大 n / 权重网格 / 白名单落地后回归；256/512 真重建（需改常量另包）。

## 6. 合并与验收（WP-125）
- [x] WP-126/127 done
- [x] §5 含真引擎算法表 + 切块表；算法结论=不足以下产品推荐（硬门槛遵守）
- [x] 未改 src；未 commit；未覆 `out/`；validate_all ok（子包自报）

## 7. 不在本包
落地改准入/切块/RRF；换模；上云；UI；kept_from 实现。

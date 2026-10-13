# WP-120-research · 数据源白名单 · 切块 · 算法定口径（WP-121 方案）

- 角色：backend_mid；父 WP-120（gm，approver=andy）；本包 WP-121（in_progress）。
- 规则：**只读，仍非落地**；不改 `src/`、`tests/`；脚本仅 `.tmp/wp120/`；不 commit；不碰 `windows-*.yml`。
- 时间：2026-10-13 21:2x HKT。
- 继承：WP-116 已封（差因/三态体积）；本包把「排除清单」改成 **白名单只要什么**，并补切块+算法实测。

## 1. Andy 锁定口径（2026-10-13）
1. **仍非落地**；整批方案未定全前不开实现。
2. **检索质量 = 数据源 + 切块 + 算法**；算法**未测不能当产品开干**。
3. **数据源=白名单（只要什么），不是黑名单**：
   - **稠密只要三态**：`user`、助手消息、助手思考。
   - **稀疏 = 三态 + `tool_call`**（工具调用=意图）。
   - 其他（含 `tool_result`、reminder、control、media…）**根本不要**。
   - session 以后加什么，search **只取白名单**；加再多不脏。
4. WP-116 附录（compact/`kept_from`）不抢本包主线。

## 2. 子包
| 包 | owner | 范围 | 产物 |
|---|---|---|---|
| WP-122 | ide | 主力：白名单对照 + 切块小样本 + 算法小样本；出口径草案素材 | `WP-120-research-ide.md` |
| WP-123 | session | 白名单语料切片 + 可复现小样本 query 集（无正文泄露）供 IDE | `WP-120-research-session.md` |

顺序：WP-123 尽快给切片/query（可与 WP-122 A 并行）；切块/算法实测依赖 query 集。中层合并 §6 后 deliver WP-121。

## 3. 验收细则

### A · 数据源白名单（IDE 主 + Session 切片）
**思路：只写「要什么」**，报告里禁止用「排除清单」当主结论（对照表可写现状多了什么，但口径草案必须是白名单）。

| 腿 | 白名单（只要） |
|---|---|
| 稠密 | `item/user`；`item/assistant`+非 reasoning；`item/assistant`+reasoning |
| 稀疏 | 上列三态 + `item/tool_call` |

验收：
- 代码锚点：现状 `Policy::Final` / `derive` vs 白名单缺口（文件:行）。
- 体积对照（复用 wp102/wp116 样本）：现状 vs 白名单 条数/token/chunk（granite±cl100k）。
- 一句产品话术草案：「search 只取 {白名单}；session 新增 kind 默认不可搜，除非改白名单」。

### B · 切块（IDE，可复现小样本数字）
现状锚点：`DENSE_CHUNK_TOKENS=448`、`chunk.rs` head+tail anchor；稀疏另有 trimmer。

在**同一白名单语料**上，只读脚本对照 ≥2 档（不改 src；可复制 chunk 逻辑到 `.tmp/wp120/`）：
| 档 | 建议 |
|---|---|
| 现状 | 448 + 现有 anchor |
| 备选 A | 更短（如 256）或更长（如 512 边界内） |
| 可选 B | 按角色/段边界切（仅若易只读近似） |

每档报：chunk 数、估嵌入墙钟（用 WP-103 吞吐外推即可）、相对召回/覆盖的**定性+小样本**（用 WP-123 query：命中率或 top-k 含金标签行比例）。真模型仅在外推不够时、`run_limited` 护栏下跑。

### C · 算法（IDE，可复现小样本数字 · 硬门槛）
只读摸底：`ranking.rs` RRF_K=60、稀疏多层、dense ANN、融合与 band。

**必须有数字**：固定 query 集（WP-123，≥8 条，含：纯对话 / 工具意图 / 失败排查类【预期白名单后应变弱】/ 短 user）。对每条报：
- 稀疏-only / 稠密-only / 融合（现状 RRF）的 top-k 是否含「金」行（金=预标 seq 或 kind 桶，**不落正文**）。
- 至少一档对照：关 dense、或改 RRF 权重/只比一层（脚本侧模拟即可）。

无小样本表 → **不得**写「推荐上线某算法」。可写「不足以下结论，待加测」。

### D · Session（WP-123）
- 白名单聚合切片：`.tmp/wp120/out/whitelist_slices.json`（复用 wp116 映射裁定）。
- 小样本 query 集：`.tmp/wp120/out/queries.json`——每条含 `id`、短 query 文本（可合成/脱敏）、`gold`（session_id 可哈希、seq、kind 桶）、`intent` 标签；**无长正文**。
- 方法可复现命令写入产物。

## 4. 证据与护栏
- 标注实测/推断/公开；样本注明 5 天局限。
- 真模型：`.tmp/wp102`/`wp116`/`wp120` 的 `run_limited.sh`；LEAN；不测 batch=8。
- 不改 src/tests；validate_all ok。

## 5. 发现（回填）
来源：WP-123 done + WP-122 done（2026-10-13）。样本 5 天；外推不代表长期。

### 5.1 白名单对照与体积
**口径（只要什么）：** 稠密=`user` / 助手消息 / 助手思考；稀疏=三态+`tool_call`。`tool_result` 等不进白名单。

| 层 | 条数 | cl100k（Session） | granite（IDE） |
|---|---:|---:|---:|
| final / 现状稠密 | ~3388 | 1.58M | 1.55M |
| dense 白名单 | 932 | 0.54M（相对 final **33.8%**） | 0.50M（**−67.6%**） |
| sparse 白名单 | 2160 | 0.62M（**39.4%**） | 0.59M |
| sparse−dense | — | 87.7k（全 tool_call） | — |

缺口锚点：稠密 Final 多 Did/Outcome（`corpus.rs:188-194`）；稀疏 `derive` 宽于白名单（`derive.rs:89-96`）。

**话术草案：** search 只取白名单；session 新增 kind 默认不可搜，除非改白名单。

### 5.2 切块小样本
白名单稠密 932 行；外推 WP-103 吞吐，未再跑真模型。

| 档 | chunks | @10c/s | @7.8 LEAN |
|---|---:|---:|---:|
| 448+anchor（现状） | 2042 | 204 s | 262 s |
| 256+anchor | 2953 | 295 s | 379 s |
| 512 无 anchor | 1602 | 160 s | 205 s |

倾向（非落地裁定）：默认维持 448；512 可作省嵌入备选待真引擎验。

### 5.3 算法小样本
正式 query n=10（含 failure_triage×2 `expect_degraded`）。offline BM25+hash128，RRF_K=60；**非**生产 ANN。

- exact-seq hit@10：**各模式 0/10**（gold 为长度种子抽 seq，与 query 词面常无交集 → 不足当相关性评测）。
- 结构性 bucket@10：对话类偏高；failure_triage 在白名单上 tool_result 桶命中 0，full5 诊断部分命中 → 与 expect_degraded 方向一致。

**硬门槛：** 有表，**不得**写产品算法推荐（权重/关 dense/换融合）。待真引擎 + 相关性金标。

### 5.4 口径草案（白名单表述）
search 只取 {稠密三态；稀疏=三态+tool_call}；新增 kind 默认不可搜，除非改白名单。

### 5.5 推荐组合 / 不做 / 待裁定
- **可进后续落地讨论（有体积+代码证据，非算法调参）：** 白名单准入；切块默认 448。
- **不做（本包）：** 改 src；依 offline 表上线算法变更；换模/上云。
- **待裁定：** 真引擎 hit@k 或相关性金标；kept_from/UI 另包。

## 6. 合并与验收（WP-121）
- [x] WP-122/123 done
- [x] §5 含白名单口径草案 + 切块表 + 算法表（有数字；算法结论=不足以下推荐）
- [x] 未改 src；未 commit；validate_all ok（子包自报）

## 7. 不在本包
落地改准入/切块/算法；换模；云；UI；kept_from 大刀实现。

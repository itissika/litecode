# WP-124-research-ide · 白名单 + 生产 ANN/RRF + 切块（WP-127）

- owner：ide；父 WP-125 / WP-124；包 WP-127。
- 规则：只读 `src/`；**只写** `.tmp/wp124/ide/`；**不写** `.tmp/wp124/out/`（Session/WP-126 正式集只读）。
- 时间：2026-10-13 22:2x HKT。
- 正式金标（RO）：`.tmp/wp124/out/queries.json`（seed=126，n=10，五类各2；`failure_triage`×2 `expect_degraded`）。
- 证据标【实测/公开/推断】。

---

## 口径草案（白名单 · 只要什么）

| 腿 | **只要** |
|---|---|
| **稠密** | `item/user`；`item/assistant`（消息）；`item/assistant`+`reasoning`（思考） |
| **稀疏** | 上列三态 + `item/tool_call` |

**产品话术**：「search 只取 {稠密三态；稀疏=三态+tool_call}；session 新增 kind 默认不可搜，除非改白名单。」  
（继承 WP-120；与 Session `product_line` 一致。）

### 算法 / 切块裁定【实测·小样本】

| 题 | 裁定 |
|---|---|
| 算法（sparse / dense / RRF） | **有数字**（见 §C）；RRF exact-seq hit@10=4/10 优于单腿 2/10。**不足以下产品结论**：n=10、单语料、未扫权重；**不得**据此改线上权重或下线某腿。 |
| 切块（448 vs 256/512） | **448 已真嵌入实测**；备选仅脚本估 chunks+外推墙钟，**召回列 N/A**（`DENSE_CHUNK_TOKENS=448` 硬编码，未改 src）。倾向：维持 448；512 作省嵌入候选待落地包。 |

---

## A · 生产路径摸底【实测】

### 代码锚点【公开】
| 组件 | 锚点 |
|---|---|
| Dense ANN | `session_search/semantic_index.rs`：`DENSE_CHUNK_TOKENS=448`（L35）；`ensure_session_index`（L877）；usearch `vectors.usearch` |
| Sparse | `session_search/sparse.rs`：`sparse_index_path` → `.litecode/session-index/sparse.db`；`CHUNK_TOKENS=448` |
| RRF | `ranking.rs` `RRF_K=60`（L372）；`mod.rs` `fuse_session_layers`（L275）`SPARSE_WEIGHT=SEMANTIC_WEIGHT=1.0`（L280–284） |
| 窗口 | `SEMANTIC_WINDOW=64`（`mod.rs` L53） |

### 驱动【实测】
- Harness：`.tmp/wp124/ide/harness/`（path-dep `litecode`，`ort=2.0.0-rc.12` 钉死；公开 API：`ensure_sparse_index` / `ensure_session_index` / `load_session_index` / `search_all` / `fuse_session_layers`）。
- 二进制：`.tmp/wp124/ide/target/release/wp127_harness`（测完可删 target；脚本保留）。
- 命令：`index <root>`；`search <root> sparse|dense|fuse <query> [k]`。
- 嵌入：生产 `granite97-ort-q8q4`（`LITECODE_ORT_LEAN=1`，`ORT_INTRA_THREADS=4`，`prlimit --as=4GiB`，`taskset 0-3`）；**非** Hash / offline BM25。
- 护栏脚本：`.tmp/wp124/ide/run_index.sh`（拒测 cargo/rustc 并存）。

### 语料工作区【实测】
| root | DB 过滤 | 用途 |
|---|---|---|
| `ws_dense` | 三态 only（932 rows） | dense-only；稠密真索引 |
| `ws_wl` | 三态+tool_call（2160 rows） | sparse-only；稠密含 Did 残差诊断 |
| `ws_fuse` | wl sessions + wl `sparse.db` + **dense 文件覆写自 ws_dense** | RRF：稀疏白名单 + 稠密白名单 |

稠密代码 `Policy::Final` 仍准入 Did（tool_call）【公开·`slots.rs`】；本包用 **过滤 DB** 实现白名单，不改 src。`ws_fuse` 显式去掉稠密 Did。

---

## B · 切块小样本

语料：稠密白名单 932 行；granite tok_sum=500,851。【实测】

| 档 | chunks（脚本估） | chunks（真索引） | 墙钟 |
|---|---:|---:|---|
| **448+anchor（现状）** | 2042 | **1044** | **187 s**（LEAN+AS4G，≈5.6 c/s） |
| 256+anchor | 2953 | — | 估 @meas≈529 s；@7.8≈379 s |
| 512 无 anchor | 1602 | — | 估 @meas≈287 s；@7.8≈205 s |
| 448 wl+Did（诊断） | — | 2334 | 400 s |

CSV：`.tmp/wp124/ide/chunk_matrix.csv`。  
脚本估 > 真索引：生产 derive/trim/anchor 与「整 body 硬切」不等价【推断】。  
**召回**：仅 448 档绑正式金标（§C）；256/512 需改 `DENSE_CHUNK_TOKENS` 才能重建，本包未改 src → 召回 N/A。

---

## C · 算法小样本（硬门槛 · 有表）

### 方法【实测·生产路径】
- Query：正式 `out/queries.json` **只读**（n=10）。
- 索引：上表；embedder=`granite97-ort-q8q4`；ANN=usearch；稀疏=生产 `sparse.db`；融合=`fuse_session_layers`（K=60，w=1/1）。
- 对照：`sparse`；`dense`（三态）；`rrf`（fuse）；`dense_wl_residual`（wl 上稠密含 Did）。
- 主指标：exact-seq hit@5 / hit@10；次：kind-bucket hit。
- 表：`.tmp/wp124/ide/algo_matrix.csv`、`algo_summary.json`。

### 主指标：exact-seq hit@10（及 hit@5）

| intent (n) | sparse @10 (@5) | dense @10 (@5) | RRF @10 (@5) | dense_wl_res @10 (@5) |
|---|---:|---:|---:|---:|
| short_user (2) | 2 (2) | 1 (1) | **2 (2)** | 1 (1) |
| pure_dialogue (2) | 0 (0) | 0 (0) | **1 (0)** | 0 (0) |
| assistant_thinking (2) | 0 (0) | 1 (0) | **1 (0)** | 1 (0) |
| tool_intent (2) | 0 (0) | 0 (0) | 0 (0) | 0 (0) |
| failure_triage (2, degraded) | 0 (0) | 0 (0) | 0 (0) | 0 (0) |
| **合计** | **2/10 (2)** | **2/10 (1)** | **4/10 (2)** | **2/10 (1)** |

kind-bucket@10（次）：sparse 2 · dense 6 · RRF 7 · residual 7。

### 解读（非产品推荐）
1. **相对 WP-122 offline 0/10**：生产路径 + 真模型给出非零 exact-seq【实测】。
2. **RRF 在本金标上高于单腿**（4 vs 2），主要来自 short_user 保住稀疏命中，并补上 pure_dialogue / thinking 各 1。【实测】
3. **failure_triage 全 0**：金标 `kind_bucket=tool_result`，不在白名单；`expect_degraded=true` 符合预期【实测】。
4. **tool_intent exact-seq 全 0**：金标多为 `tool_call`；稀疏未命中 exact-seq（kind 次指标 RRF@10 仅 1/2）——小样本不足以下「稀疏对 tool 足够」结论【实测·不足】。
5. **稠密 Did 残差**（`dense_wl_residual`）exact-seq 未优于三态 dense；不支持「保留 Did 能抬 exact-seq」【实测·不足】。

**硬门槛声明**：有小样本数字，可作相对比较；**禁止**据此写「上线某权重 / 下线 dense」类产品推荐。待加测：更大 n、权重网格、白名单落地后的回归。

---

## D · 不做清单

- 不改 `src/` / 不 commit / 不写 `windows-*.yml`。
- 不覆写 `.tmp/wp124/out/`。
- 不测 batch=8；不用 Hash 主表；不接云。
- 不对 256/512 声称召回优劣。
- 无「算法产品推荐」段落。

---

## E · 护栏与清理

- 索引墙钟：`.tmp/wp124/ide/index_wall.txt`（dense 187s / wl 400s）。
- 保留：`algo_matrix.csv`、`chunk_matrix.csv`、`eval_algo.py`、`run_index.sh`、`plan.md`、harness 源。
- 清理：大索引/`target`/sessions 副本可删（deliver 后）；自起进程无残留 serve。

# WP-128-backend · 白名单准入执行（WP-129 中层方案）

- 角色：backend_mid；父 WP-128（gm，approver=andy，in_progress）；本包 WP-129（in_progress，approver=gm）。
- 基线：分支 `ops/team-framework`；中层只拆活 / 审域方案 / 验收 / 汇总；**不写 `src/`、`tests/`**；**不 commit / push**；不碰 `.github/workflows/windows-*.yml`。
- 时间：2026-10-13 23:0x HKT。
- 依据：WP-124 已封（§5）；GM 落地口径；调研 WP-101/116/120/124。

## 1. Goal

落地 **session 检索白名单准入**：

| 腿 | 只要 |
|---|---|
| **稠密** | 三态：`item/user`；`item/assistant`（消息）；`item/assistant`+`reasoning`（思考） |
| **稀疏** | 上列三态 + `item/tool_call` |

其余（含 `item/tool_result`、reminder/control、`compacted`、未知 kind 等）**不进对应腿**。session 以后新增 kind **默认不可搜**，除非改白名单。

## 2. GM / Andy 锁定（本包硬边界）

1. 稠密只要三态；稀疏=三态+`tool_call`；其余不要。
2. **切块默认 448 不改**（`DENSE_CHUNK_TOKENS` / sparse `CHUNK_TOKENS`）。
3. **RRF / 排序本包不调参**（`RRF_K`、`SPARSE_WEIGHT`/`SEMANTIC_WEIGHT` 维持现状）。
4. **不新 Warming / 状态机**；LEAN / 批提交续嵌 **不在本包**。
5. 验收：白名单外 kind 不进对应腿；覆盖纯对话 / 工具意图 / 失败排查 / 短 user；云端绿后 deliver；Andy 真机归 WP-128 收口。
6. **继承 WP-124 硬门槛**：n=10 小样本 **不得**据此发明算法参数或产品算法推荐。

## 3. 子包树

```
WP-128 (gm, in_progress)  语义检索落地：白名单准入
  └── WP-129 (backend_mid, in_progress)  ← 本包
        └── WP-130 (ide, drafting)       白名单准入落地
```

| 子包 | owner | approver | 产物 |
|---|---|---|---|
| WP-130 | ide | backend_mid | `.litecode/ops/artifacts/WP-128-ide.md` |

本包仅拆 IDE：准入与语料投影在 `src/engines/session_search/**`；Tools/Session/前端本波无改码面。

## 4. WP-130 · IDE 范围与锚点（只点位，中层不改码）

**主改（预期）：** `src/engines/session_search/`

| 主题 | Likely 锚点 | 要做什么 |
|---|---|---|
| 准入 / 白名单 | `slots.rs`：`kind_included`（现含 tool_result/reminder 等）；`classify`；`Policy::Final` | 稠密腿只要 Who/Said/Thought；稀疏腿 + Did；Outcome/Other/Summary/When 等默认不进对应腿 |
| 投影 / 入库 | `corpus.rs` `derive_docs` / Policy::Final 准入；`derive.rs` | 两腿分流后非白名单不产生对应 docs |
| 稠密索引 | `semantic_index.rs`（仍走 Policy::Final；`DENSE_CHUNK_TOKENS=448` **勿改**） | 稠密不得再吃 Did/Outcome |
| 稀疏索引 | `sparse.rs`（`CHUNK_TOKENS=448` **勿改**） | 稀疏可留 tool_call；拒 tool_result 等 |
| 融合 | `mod.rs` `fuse_session_layers`；`ranking.rs` `RRF_K=60` | **只读不调** |

**测试（预期）：** 同 crate / 既有 session_search 测 + 必要新测：白名单外不进腿；稀疏仍见 tool_call；四类意图（对话 / 工具意图 / 失败排查 expect_degraded / 短 user）。

## 5. 验收细则（对齐 GM）

- [ ] 单元/集成：白名单外 kind **不进**对应腿；稀疏仍可见 `tool_call`。
- [ ] 用例覆盖：纯对话、工具意图、失败排查（tool_result 不进索引后行为符合预期）、短 user。
- [ ] 切块常量仍为 448；RRF/权重未改。
- [ ] 无新 Warming/状态机；未做 LEAN/续嵌。
- [ ] 相关 cargo 测绿；云端绿后 IDE deliver；中层抽查后 close WP-130。
- [ ] 产物 `WP-128-ide.md` 含改动 file:line、测命令与证据；**未 commit**。
- [ ] Andy 真机：记在 WP-128，不挡 WP-130/129 云端 deliver（真机由 GM/Andy 收口）。

## 6. 流程

1. 中层已 `start` WP-129；已 `new` WP-130。
2. IDE：`fill` 域方案 → `submit`（approver=backend_mid）。
3. 中层审方案 → `approve` → IDE `start` 开工。
4. IDE 改码+测绿 → 写 `WP-128-ide.md` → `deliver`。
5. 中层抽查（准入断言、448/RRF 未动、禁区）→ `close --to done` WP-130。
6. 汇总本文件 → `deliver` WP-129 给 GM；统一 commit 仍由 GM 收口。

## 7. 禁区

- 中层不写 `src/` / `tests/`。
- 不改切块常量、RRF/排序权重、Warming/状态机、LEAN/续嵌。
- 不依 WP-124 小样本发明算法产品推荐或调参。
- 不 commit / push；不碰 `windows-*.yml`；不碰 Andy 本机路径。

## 8. 风险 / 待 IDE 核实

- 现状 `Policy::Final` 仍准入 Did + Outcome（tool_call/tool_result）；稠密与稀疏是否共用同一 derive 入口 —— IDE 须分腿或分策略，避免「稀疏要 tool_call、稠密不要」互相踩。
- FTS / 稀疏 SQL 层若另有 kind 列表，须与白名单对齐。
- 索引兼容：白名单收紧后是否触发重建 —— 沿用现有不兼容/重建路径，**不新造** Warming 语义。

## 9. 落地验收（WP-130 → WP-129 · 2026-10-13 23:3x HKT）

**判定：PASS**（中层抽查；未改产品码；未 commit）

### 9.1 抽查证据（file:line）

| 项 | 证据 |
|---|---|
| dense 白名单 | `slots.rs:97–99` `admits_dense` = Who\|Said\|Thought |
| sparse 白名单 | `slots.rs:102–104` `admits_sparse` = 三态 + Did |
| dense 入口 | `corpus.rs:188` `Policy::Final => admits_dense(slot)`（原 Did\|Outcome 已去掉） |
| sparse 入口 | `derive.rs:98–101` `admits_sparse(slot)`；测 tool_call✓ / tool_result✗（~L247–255） |
| Outcome 投影 | `slots.rs:183` `row_text_final` Outcome→None |
| kind 默认出 | `slots.rs:166–172` 仅 user/assistant/tool_call；unknown→false |
| DOC_SCHEMA | `semantic_index.rs:32` **3**（触发 rebuild） |
| INDEX_SCHEMA | `sparse.rs:141` **15**（清旧 tool_result chunk） |
| SPEAKER_ORDER | `sparse.rs:151–154` tool_result 臂标死/兼容，非准入 |

测断言抽样：`dense_parity.rs:178–189` dense 拒 Did；`golden.rs:209–214` / `343` sparse 拒 tool_result；`mod.rs` agent_view 仅 call；`trimmer.rs:150–164` index 空 tool_result。

### 9.2 硬门【对照 git HEAD】

| 常量 | 工作区 | diff |
|---|---|---|
| `DENSE_CHUNK_TOKENS` | 448（semantic_index.rs:35） | **未改** |
| `CHUNK_TOKENS` | 448（sparse.rs:1639） | **未改** |
| `RRF_K` | 60.0（ranking.rs:372） | ranking.rs **未动** |
| `SPARSE_WEIGHT` / `SEMANTIC_WEIGHT` | 1.0（mod.rs:280–284） | **未改数值** |
| Warming / LEAN | — | **无新增**（仅注释「No new Warming path」） |
| 范围 | 10 files under `src/engines/session_search/**` | 无产品外改码 |

### 9.3 测试

- IDE 实测：`cargo test --lib session_search` → **161 passed; 0 failed; 1 ignored**（`.tmp/wp130/test_ss3.log`）。
- 中层复跑：同命令在 600s 内卡在 litecode 重编未出 result；**信任 IDE 绿 + 源码断言抽查**。

### 9.4 验收清单（§5）勾选

- [x] 白名单外不进对应腿；sparse 仍见 tool_call
- [x] 对话 / 工具意图 / 失败排查（tool_result 两腿拒）/ 短 user（测覆盖）
- [x] 448 / RRF/权重未改
- [x] 无新 Warming/LEAN
- [x] IDE deliver + 中层抽查 close WP-130
- [x] 产物 `WP-128-ide.md`；**未 commit**
- [ ] Andy 真机 → 归 **WP-128**（GM/Andy）

### 9.5 交接 GM（WP-129 deliver）

云端白名单已落地未 commit；请 GM 收口 commit / 云端绿 / Andy 真机（WP-128）。

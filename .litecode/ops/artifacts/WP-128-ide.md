# WP-128-ide · 白名单准入落地（WP-130）

- owner：ide；approver：backend_mid；父：WP-129 / WP-128。
- 时间：2026-10-13 23:2x HKT。
- 规则：只改 `src/engines/session_search/**` + 测；**未 commit**；未碰 `windows-*.yml`；448 / RRF / 权重未动；无新 Warming/LEAN。
- 方案：`.tmp/wp130_plan.md`。

## 1. 口径（已落地）

| 腿 | 只要 |
|---|---|
| **稠密** | Who / Said / Thought（`item/user`；`item/assistant` 消息；`reasoning`） |
| **稀疏** | 三态 + Did（`item/tool_call`） |

其余（`tool_result` / reminder / control / compacted / 未知）**不进对应腿**；新 kind 默认不可搜。

## 2. 改动 file:line

| 文件 | 要点 |
|---|---|
| `slots.rs` | `admits_dense` / `admits_sparse`（~L97–104）；`kind_included` 收窄为 user/assistant/tool_call（~L166–172）；`row_text_final` Outcome→None（~L181–187）；模块文档改白名单 |
| `corpus.rs` | `Policy::Final` 准入 → `admits_dense(slot)`（~L185–190） |
| `derive.rs` | `derive_row` 加 `admits_sparse(slot)`（~L95–101）；测：tool_call 进 / tool_result 拒 |
| `semantic_index.rs` | `DOC_SCHEMA` 2→**3**（L32）触发既有 rebuild；注释改三态；**`DENSE_CHUNK_TOKENS=448` 未改**（L35） |
| `sparse.rs` | `INDEX_SCHEMA` 14→**15**（全量 rebuild 清旧 tool_result chunk）；`SPEAKER_ORDER_SQL` tool_result 臂标死/兼容（不调 RRF）；**`CHUNK_TOKENS=448` 未改** |
| 测试 | `slots` / `derive` / `golden` / `dense_parity` / `relevance` / `mod` agent_view / `trimmer` 断言对齐新白名单 |

## 3. 448 / RRF 证据【实测】

```
semantic_index.rs:35  const DENSE_CHUNK_TOKENS: usize = 448;
sparse.rs:1639        const CHUNK_TOKENS: usize = 448;
ranking.rs:372        pub const RRF_K: f64 = 60.0;
mod.rs:280–284        SPARSE_WEIGHT = SEMANTIC_WEIGHT = 1.0;
```

`git diff` 未改上述常量行。

## 4. 测试【实测】

```text
HOME=.tmp/home CARGO_TARGET_DIR=.tmp/wp130/target \
  timeout 600 cargo test -j1 --lib session_search -- --test-threads=1 </dev/null
→ ok. 161 passed; 0 failed; 1 ignored
```

覆盖：对话（Who/Said/Thought）· 工具意图（sparse 收 tool_call、dense 拒）· 失败排查（tool_result 两腿皆拒）· 短 user。Andy 真机归 **WP-128**。

## 5. 不做清单

调 RRF/切块；新 Warming；LEAN/续嵌；UI；commit/push；windows workflow。

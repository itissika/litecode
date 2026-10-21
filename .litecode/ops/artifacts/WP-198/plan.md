# WP-198 方案 — edit / apply_edits 精简（硬清）

**owner：** backend_mid  
**分支：** `feat/tool-contract-ptc`（不合 main）  
**串行：** **WP-197 重型槽优先**；本包可批可写码，**禁止与 197 同时重 cargo**。  
**依据：** brief；Andy+GM 定稿硬清

---

## 1. 目标契约（唯一）

```ts
tools.edit({ path, old_string, new_string }) -> EditResult
tools.apply_edits(edits: Array<{ path, old_string, new_string }>) -> { results: EditResult[] }
// 串行；单项失败跳过继续

type EditResult = { ok: boolean; path: string; message: string };
```

- 入参 object bag（与 Code 习惯一致）。
- `edit`：单文件单替换；匹配须唯一（多匹配失败，指引写进 `message`）。
- `apply_edits`：数组串行；失败跳过继续；返回 `results` 与入参数组等长。
- 空 `new_string` = 删除 `old_string`（保留现行语义）。
- 有用失败指引（多匹配、未找到、权限等）**只**进 `message` 文本，不堆 details 树。

---

## 2. 硬清（一行不留）

| 砍 | 说明 |
|---|---|
| `replace_all` | 参数与行为全删 |
| `expected_hash` | 首期不做；相关 stale 结构化出口删 |
| 旧多块 API | 单工具 `edits[]` 多块同 path 形态删除 |
| 双出口 | 半成功 resolve vs 全失败 throw → 统一 resolve + `ok`/`message` |
| 结构化报告 | `blocks`/`applied`/`failed`/`details`/`file.hash` 等旧出参全删 |

可复用底层：精确匹配、CRLF/LF、BOM、写盘一次——**内部用**，对外只暴露 EditResult。

---

## 3. 分工

| 域 | 职责 |
|---|---|
| **Runtime** | 新契约实现 + 硬删旧 edit API/测；默认绑定 `edit`+`apply_edits`；dig-runtime-impl |
| **Tools** | description + SDK fixture + 单轨教法；扫残留 |
| **mid** | 与 WP-197 串槽；交叉审；grep；acceptance/diff-summary；commit/push；deliver |

顺序：方案批后 → Runtime（可与 197 实现期并行写码，**cargo 必须等 197 测完放槽**）→ Tools 对齐 → mid 收口。

---

## 4. 验收

1. Code 面仅 `edit` + `apply_edits`；grep 无旧契约残留（`replace_all`/`expected_hash`/blocks 出参等）。
2. 失败跳过续跑；`message` 保留有用指引。
3. 相关 cargo 绿；`artifacts/WP-198/{acceptance,diff-summary}.md`；push 实验分支；未合 main。

---

## 5. 拍板

| # | 议题 | 默认 |
|---|---|---|
| Q1 | 工具名 `edit` / `apply_edits` | 采用 |
| Q2 | 砍 replace_all + expected_hash（首期） | 是 |
| Q3 | 统一 resolve + EditResult，无双出口 | 是 |
| Q4 | 与 WP-197 串行重型槽（197 优先） | 是 |
| Q5 | 不合 main | 是 |


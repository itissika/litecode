# WP-199 方案 — grep / glob / read 精简（硬清冗余）

**owner：** backend_mid  
**分支：** `feat/tool-contract-ptc`（不合 main）  
**基线：** HEAD `25a15e1`  
**依据：** brief；Andy+GM 定稿硬清  
**内存：** available ≥ 2GiB 即可 cargo（中层串行放行）

---

## 1. 目标契约（唯一）

```ts
tools.grep({ pattern, path?, glob?, literal?, ignore_case?, limit? })
  -> { hits: Array<{ path, line, text }>; message: string }

tools.glob({ pattern, path?, limit? })
  -> { paths: string[]; message: string }

tools.read({ path, start_line?, limit? })
  -> { path, start_line, lines: string[]; message: string }
```

- 入参 object bag。
- **行号：** 全工具 1-based；grep `hits[].line`；read 保留入参/出参名 **`start_line`**（禁止改成 offset）。
- 一次有界（limit）；搜到/没搜到；触顶/原因/截断说明进 **`message`**。
- `hits[].text`：该行有界摘要（截断标注进 text 或 message，不另堆字段树）。
- `read.lines`：字符串数组（该页行内容）；续读用更大 `start_line`，**无 cursor**。

---

## 2. 硬清（一行不留）

| 砍 | 含 |
|---|---|
| 分页方言 | `cursor` / `stale_cursor` / `page` / `page.next` / `has_more` |
| 枚举/树 | 多枚举 `reason`、`searched`、`base`、matches 旧字段树 |
| grep 旧出参 | `matches`/`start_line`/`end_line`/`line_text`/`line_truncated` 等 → 统一 `hits[{path,line,text}]` |
| glob 旧出参 | `base` workspace|absolute 双态等冗余；paths 一律工作区相对（除非实现必须绝对——默认相对，绝对只进 message 说明） |
| read 旧面 | `end_line`、`file.hash`/`expected_hash`、`lines[].line_truncated` 结构、`reason` 枚举、`page.*` |
| 未列入参 | brief 未列的旋钮（如 `include_ignored`）默认砍；若砍后无法搜 gitignore 外文件，message 说明，不另造方言 API |

语言：教法通用易懂，禁止再造方言术语。

---

## 3. 分工

| 域 | 职责 |
|---|---|
| **Runtime** | 三工具实现+硬删旧契约/测；默认绑定不变名；dig-runtime-impl |
| **Tools** | description + SDK fixture + 单轨教法；扫残留方言 |
| **mid** | 串槽、交叉审、grep 验收、acceptance/diff-summary、commit/push、deliver |

顺序：Runtime 硬清 → Tools 对齐 → mid 收口。cargo 独占串行，门槛 ≥2GiB。

---

## 4. 验收

1. 三工具按上契约；grep 活跃面无旧方言残留（cursor/stale_cursor/reason 枚举/searched/base/page 等）。
2. 行号统一 1-based；read 仍 `start_line`；教法通用易懂。
3. 相关 cargo 绿；`artifacts/WP-199/{acceptance,diff-summary}.md`；push 实验分支；未合 main。

---

## 5. 拍板

| # | 议题 | 默认 |
|---|---|---|
| Q1 | 出参形态按 brief（hits/paths/lines+message） | 采用 |
| Q2 | 硬清 cursor/page/reason 树等 | 是 |
| Q3 | read 保留 `start_line`，不改 offset | 是 |
| Q4 | brief 未列旋钮（含 include_ignored）默认砍 | 是 |
| Q5 | 不合 main | 是 |


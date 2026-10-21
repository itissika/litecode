# WP-199 Runtime 实现笔记 — grep / glob / read（硬清）

> **作者：** Runtime（Grok Bot 执行子代理）  
> **时间：** 2026-10-22 Asia/Hong_Kong  
> **分支 / HEAD 基线：** `feat/tool-contract-ptc` @ `25a15e1`（实现**未** commit；工作树有改动）  
> **依据：** `brief.md`、`plan.md`（Q1–Q5 全认）；Andy+GM 硬清口径  
> **硬约束遵守：** **199 cargo 绿**（61 tools + 20 phase2/matrix/baseline；见 cargo-grep-glob-read.txt）；**未 commit/push**；**未改** Tools SDK fixture（`src/tool/fixtures/run_code_sdk.*`）

---

## 1. 关闭/替换层（一句话）

默认绑定名不变（`grep` / `glob` / `read`）。出参统一为 brief 契约：`hits`/`paths`/`lines` + `message`；成功态 **空 content**（与 write/edit present 同口径）。分页方言、多枚举 reason、searched/base、include_ignored、read.end_line / file.hash / lines[].line_truncated 全部删除。

---

## 2. 行为（对照 plan §1–2）

| 工具 | 入参 | 出参 |
|---|---|---|
| **grep** | `{pattern, path?, glob?, literal?, ignore_case?, limit?}` | `{hits:[{path,line,text}], message}` |
| **glob** | `{pattern, path?, limit?}` | `{paths, message}` |
| **read** | `{path, start_line?, limit?}` | `{path, start_line, lines:string[], message}` |

- 行号全 **1-based**；read 保留 **`start_line`**（未改 offset）。
- 一次有界（limit）；触顶/空结果/截断说明只进 **`message`**。
- `hits[].text`：有界一行摘要（超长加 `...`）。
- `glob.paths`：默认工作区相对；工作区外绝对路径时 message 注明。
- `read.lines`：字符串数组；续读用更大 `start_line`，无 cursor。
- `case_sensitive` → **`ignore_case`**（默认 false）；`include_ignored` **砍**（默认 Search 过滤；点名被排除路径时 message 说明）。

底层仍复用 lexical walk / glob walker / 虚拟 session 投影；仅契约面硬清。

---

## 3. 删除清单（硬清）

| 类 | 已删/已改 |
|---|---|
| 分页方言 | `cursor` / `stale_cursor` / `page` / `page.next` / `has_more`（出参） |
| 枚举/树 | 多枚举 `reason`、`searched`、`base`、旧 matches 字段树 |
| grep | `matches`/`start_line`/`end_line`/`line_text`/`line_truncated`/`include_ignored`/`case_sensitive` → `hits[{path,line,text}]` + `ignore_case` |
| glob | `base`/`page`/`reason`/`include_ignored`/`cursor` |
| read | `end_line`、`file`/`hash`/`expected_hash`、`lines[].{number,text,line_truncated}`、`page`、`reason` |
| descriptions | 三份 md 重写为通用易懂教法 |
| 测 | unit / phase2 / contract_matrix / baseline fixtures |
| SDK fixture | **未改**（Tools 收口） |

**故意保留：** 负向断言字符串（`include_ignored`/`cursor`/`matches`/`page` 等「不应出现」）、`LexicalQuery.case_sensitive` 内部字段、contract_matrix 通用 page 臂（其它工具若仍用）、probe `file_not_searched` 注释。

---

## 4. 改动文件列表（Runtime）

**重写/大改：**  
`src/tools/grep.rs`、`src/tools/glob.rs`、`src/tools/read.rs`  
`src/tools/descriptions/{grep,glob,read}.md`  
`src/tools/phase2_gate.rs`  
`src/tool/contract_matrix.rs`  
`src/tools/baseline/mod.rs`  
`src/tools/baseline/fixtures/{grep,glob,read}__{success,empty,truncation}.json`  

**顺带对齐 WP-198 残留（否则 baseline 卡死）：**  
`edit__success.json` message → `replaced`；`edit__empty.json` + expect_shape 改为 ok=false resolve。

**新增：**  
`.litecode/ops/artifacts/WP-199/dig-runtime-impl.md`（本文件）  
`.litecode/ops/artifacts/WP-199/cargo-grep-glob-read.txt`

---

## 5. cargo（已跑）

```bash
LC_MIN_MEM_GIB=2 CARGO_BUILD_JOBS=2
cargo test --lib -- tools::grep:: tools::glob:: tools::read:: --test-threads=2
# EXIT_tools:0 → 61 passed

cargo test --lib -- tools::phase2_gate:: tool::contract_matrix:: tools::baseline:: --test-threads=2
# EXIT_phase:0 → 20 passed
```

tee：`.litecode/ops/artifacts/WP-199/cargo-grep-glob-read.txt`

---

## 6. grep 自检摘要（Runtime 面）

- 三工具实现/description：**无**活跃出参 `cursor`/`stale_cursor`/`has_more`/`searched`/`base`/`end_line`/`line_truncated`/`matches`/`include_ignored`/`expected_hash`。
- 命中仅负向测 / 注释 / 本地布尔 `has_more`（不进 data）/ `LexicalQuery` 内部字段。
- `src/tool/fixtures/run_code_sdk.*`：**未动**（Tools）。

建议 mid 收口再扫：

```bash
rg -n 'stale_cursor|include_ignored|"has_more"|searched|line_truncated|expected_hash' \
  src/tools/grep.rs src/tools/glob.rs src/tools/read.rs \
  src/tools/descriptions/{grep,glob,read}.md
```

---

## 7. Acceptance 素材（供 mid 收口；未 commit）

### 7.1 done_when 对照

| brief 验收 | Runtime 状态 |
|---|---|
| 三工具按契约 | ✅ |
| 行号 1-based；read 仍 start_line | ✅ |
| 活跃面无旧方言 | ✅（负向测故意保留字符串） |
| cargo 绿 | ✅ 61+20 |
| artifacts acceptance/diff-summary | ⏳ mid |
| push 实验分支 / 不合 main | ⏳ mid；本包不 commit |

### 7.2 Agent 视角最小场景

1. `tools.grep({pattern:"needle", limit:1})` → `hits.length===1`，`message` 含 Raise limit（若有更多）。
2. `tools.glob({pattern:"*.txt", limit:2})` → `paths` 工作区相对；触顶进 message。
3. `tools.read({path, start_line:1, limit:3})` → `lines` 为 string[]；未读完则 message 给下一 `start_line`。
4. 拒旧参（经 executor schema）：`cursor` / `include_ignored` / `end_line` → `invalid_input`。

### 7.3 给 mid / Tools

- Runtime 实现 + dig + cargo：**齐**；**不 commit**。
- Tools：刷 description/SDK fixture/单轨教法（`page.file.hash` 等旧句）；SDK snapshot 留给 Tools。
- 已知非本包红（若全量扫到）：`deletion_gate` 仍找已删 `edit/feedback.rs`；`phase1_gate` edit LSP / run_code undefined —— 属 198/SDK 收口，非 199 契约面。

## 8. 状态

- 实现 + dig + cargo：**完成**
- commit/push：**未做**

## 收口待命

mid：81 passed 收到；Tools 刷 SDK/教法。**Runtime 勿 commit、勿重跑重 cargo**；Tools 绿后 mid deliver。

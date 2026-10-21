# WP-199 acceptance

**口径：** Andy + GM 硬清；`grep` / `glob` / `read` 精简出参；砍 cursor/page/reason 等方言；全工具统一 **1-based** 行号。  
**口径修订（本包锁死）：** `read` 缺文件 / 非文本 → **`ToolError`**（`not_found` / `unsupported`）；空窗 / 触顶 / session-in-context → **resolve + `message`**（不另造 reason 枚举）。  
**分支：** `feat/tool-contract-ptc`（实验；不合 main）  
**方案：** `plan.md`（Q1–Q5 全认）

| # | done_when | 结果 |
|---|-----------|------|
| 1 | Code 面 grep/glob/read 出参仅精简形态；活跃面无旧契约方言 | **通过**。出参：`grep→{hits[{path,line,text}],message}`；`glob→{paths,message}`；`read→{path,start_line,lines,message}`。descriptions / PRELUDE / fixture 不教 `cursor`/`stale_cursor`/`page`/`reason`/`searched`/`base`/`include_ignored`/`end_line`/`line_truncated`/`file.hash` 作契约字段。负向测与「无 X」教法句可留。 |
| 2 | 行号心智统一；read 保留 start_line | **通过**。hits.line / read.start_line 均为 **1-based**；续读增大 `start_line`，禁止 cursor / `page.next`。limit 写死：grep 20/100、glob 1000/5000、read 2000/5000。 |
| 3 | cargo 绿 + artifacts；push；不合 main | **通过**。`cargo-grep-glob-read.txt`：**EXIT_tools:0**（61）+ **EXIT_phase:0**（20）= **81 passed**。`cargo-sdk-tools.txt`：`tool::sdk_snapshot` **2 passed**；fixture sha `12f6745f42c15531a0006de44975608df70c4590cd5850685baa734ddc62b08b`。push 实验分支；不合 main。 |

## 契约要点（已落地）

```ts
tools.grep({ pattern, path?, glob?, literal?, ignore_case?, limit? }) -> { hits: [{ path, line, text }], message }
tools.glob({ pattern, path?, limit? }) -> { paths, message }
tools.read({ path, start_line?, limit? }) -> { path, start_line, lines: string[], message }
```

- object bag；触顶 / 无结果 / 空窗指引只进 `message`
- 硬清：`cursor` / `stale_cursor` / `page` / `reason` / `searched` / `base` / `include_ignored` / `end_line` / `line_truncated` / `matches` 树
- **read 失败分流：** 缺文件/`not_found`、非文本/`unsupported` → reject `ToolError`；空窗、触顶、session 已在上下文 → resolve，说明进 `message`

## Agent 视角最小场景

1. `grep` → `hits` → `edit`（同程序）
2. `glob` → `paths` 工作区相对 → `read`
3. `read` 触顶 → `message` 提示下一 `start_line`；再调 `read`
4. `read` 缺文件 → `ToolError not_found`（非 soft ok）
5. 旧参 `cursor` / `include_ignored` → schema 拒绝或忽略（负向测覆盖）

## 同分支附注

与 **WP-197**（shell）/ **WP-198**（edit）同实验分支递进 tip；本 tip 在 `25a15e1` 之上。

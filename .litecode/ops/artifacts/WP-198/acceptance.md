# WP-198 acceptance

**口径：** Andy + GM 硬清；`edit` / `apply_edits` → `EditResult{ok,path,message}`；砍 replace_all / 首期 expected_hash / blocks 双出口。  
**分支：** `feat/tool-contract-ptc`（实验；不合 main）  
**方案：** `plan.md`（Q1–Q5 全认）

| # | done_when | 结果 |
|---|-----------|------|
| 1 | Code 面仅 edit + apply_edits；grep 无旧契约残留 | **通过**。seed 绑 `edit`；`apply_edits` companion 注入。descriptions / fixture 教法无旧出参 API（负向句说明「无 replace_all/…」除外）。实现面旧多块 API 已删；负向测与 `write.expected_hash` 故意保留。 |
| 2 | 失败跳过续跑；message 保留有用指引 | **通过**。`apply_edits` 串行；单项 `ok:false` 跳过继续；`results` 与入参等长。匹配失败等指引只进 `message`；不 throw `edit_failed`。成功短确认 `replaced`/`deleted`/`applied`。 |
| 3 | cargo 绿 + artifacts；push；不合 main | **通过**。`cargo-edit-apply.txt`：**EXIT_edit:0**（34）+ **EXIT_phase:0**（9）= **43 passed**。`contract_matrix`：`present_edit`/`present_apply_edits` 空 content（指引在 `data.message`；见 dig §9）。SDK 与 WP-197 共用已刷 fixture（`cargo-sdk-tools.txt` / `cargo-sdk-note.txt`）。push 实验分支；不合 main。 |

## 契约要点（已落地）

```ts
tools.edit({ path, old_string, new_string }) -> { ok, path, message }
tools.apply_edits({ edits: [{ path, old_string, new_string }, ...] }) -> { results }
```

- object bag；串行；同 path 后条可见先条成功写
- 砍：`replace_all`、首期 `expected_hash`、blocks/applied/failed/details、半成功 resolve vs 全失败 throw

## Agent 视角最小场景

1. `edit` → `ok` → `run`/`wait`（与 WP-197 Command 同程序）
2. 多匹配 → `ok:false` + message 指引；文件未改
3. `apply_edits` 中间 miss → `results` 等长、前后已写、不 throw
4. `new_string: ""` → 删除确认
5. 旧参 `replace_all`/`expected_hash` → `invalid_input`

## 同分支附注

与 **WP-197** 同实验分支同 tip 交付；shell 契约见 WP-197 acceptance。

# WP-198 Runtime 实现笔记 — `edit` / `apply_edits`（硬清）

> **作者：** Runtime（Grok Bot 执行子代理）  
> **时间：** 2026-10-22 Asia/Hong_Kong  
> **分支 / HEAD 基线：** `feat/tool-contract-ptc` @ `ab95a80`（实现**未** commit；工作树有改动）  
> **依据：** `brief.md`、`plan.md`（Q1–Q5 全认）；Andy+GM 硬清口径  
> **硬约束遵守：** **198 cargo 绿**（34 edit/apply + 9 phase/matrix/registry；见 cargo-edit-apply.txt）；**未 commit/push**；**未改** `src/tool/fixtures/run_code_sdk.*`

---

## 1. 关闭/替换层（一句话）

**默认绑定层 + registry 注入层：** seed/`core_configurable` 仍只绑 **`edit`**（SAFE/ALL dial 不变）；**`apply_edits` 不进设置卡**，由 registry 在见到 `edit` 时注入（Fixed）；权限/path_mode **跟 edit 绑定别名**（`permission::engine`）。旧单工具多块 `edits[]`、`replace_all`、`expected_hash`、双出口（`edit_failed` throw vs 半成功 resolve）、以及 `blocks`/`applied`/`failed`/`file.hash`/`details` 结构化出参全部删除；有用指引只进 `EditResult.message`。

### 1.1 默认 agent 首轮能否调 `edit` + `apply_edits`

| 层 | 行为 |
|---|---|
| seed / `core_configurable` | 只绑 **`edit`** |
| `build_tool_list` | 见到 `edit` 即注入 `ApplyEditsTool` |
| permission | `apply_edits` 读 **edit** 的 policy/path_mode；floor 扫 `edits[].path` 敏感路径 |
| Code host 投影 | 二者一并进 `run_code` whitelist / SDK → 首轮即可 `tools.edit` / `tools.apply_edits` |
| 关 `edit` | companion 一并消失 |

**不另补 seed 行**（与 WP-197 `wait`/`kill` 同型）。若 mid/GM 强制两行 seed，再改。

---

## 2. 行为（对照 plan §1–2）

| 工具 | 行为 |
|---|---|
| **`edit`** | `{path, old_string, new_string}` → `EditResult{ok,path,message}`。匹配须唯一；空 `new_string` = 删除；失败 **resolve**（`ok:false`），不 throw（取消/非法入参除外） |
| **`apply_edits`** | `{edits: [{path,old_string,new_string},...]}` → `{results: EditResult[]}`。**串行**；单项失败跳过继续；`results` 与入参等长 |
| **砍光** | `replace_all`、`expected_hash`、旧多块 API、双出口、`blocks`/`applied`/`failed`/`details`/`file.hash` |
| **成功 message** | 短确认 `replaced` / `deleted`（与 Tools 草稿一致）；LSP/path-risk 若有则追加换行 |

底层复用：精确匹配、CRLF/LF、BOM、原子写盘、LSP/path-risk 文案折进 `message`。

---

## 3. 删除清单（硬清）

| 类 | 已删/已改 |
|---|---|
| 实现 | 重写 `edit.rs` + `edit/{planner,matcher,engine,result}.rs`；删 `feedback.rs`；新增 `apply_edits.rs` |
| descriptions | 活文案已用 Tools `draft-*.md` 覆盖 `edit.md` / 新增 `apply_edits.md`（收口前可再刷） |
| registry | companion 注入；`builtin_code_surface_tools`；instantiate 臂 |
| permission | engine 别名；floor 扫数组 path；presets Fixed 列表测 |
| authorize | `apply_edits` summary = 路径列表 |
| reserved | `apply_edits` → `reserved_command_name_rejection` |
| 测 | `edit/tests`、`python_indent`、`phase2/3`、`contract_matrix`、baseline fixtures、`code.rs` ledger counts |
| SDK fixture | **未改**（Tools 收口）；`code_manifest` 旧教法句仍在，等 Tools 刷 |

**故意保留旧符号的位置（非契约暴露）：** 负向断言（「不应出现 replace_all/expected_hash」）、schema_validate 通用 boolean 测、write 的 `expected_hash`、retired `edit_failed + blocks` matrix 臂。

---

## 4. 改动文件列表（Runtime）

**新增：**  
`src/tools/apply_edits.rs`  
`src/tools/edit/{engine,result}.rs`  
`src/tools/descriptions/apply_edits.md`  
`.litecode/ops/artifacts/WP-198/dig-runtime-impl.md`（本文件）

**重写/大改：**  
`src/tools/edit.rs`、`src/tools/edit/{planner,matcher,tests,python_indent}.rs`  
`src/tools/descriptions/edit.md`（删 feedback.rs）  
`src/tools/mod.rs`、`src/tool/registry.rs`  
`src/config/global_db/tools.rs`  
`src/permission/{engine,floor,presets}.rs`  
`src/tool/{authorize,availability,contract_matrix,phase1_gate,resource_gate,executor}.rs`  
`src/tools/{code,phase2_gate,phase3_gate,baseline/**}.rs`

---

## 5. 测（草稿 / 未跑）

等 mid 放行 WP-197 测完后再跑。建议：

```bash
LC_MIN_MEM_GIB=2 CARGO_BUILD_JOBS=2 cargo test --lib \
  edit:: apply_edits:: phase2_chain_read_edit phase3_edit phase3_apply \
  contract_matrix -- --test-threads=2
```

覆盖点：唯一匹配失败指引、空 new_string 删除、apply_edits 串行跳过等长、拒绝 replace_all/expected_hash、companion 注入、ledger `ok(9/10)`。

---

## 6. grep 自检摘要（Runtime 面）

- `src/tools/edit*` / `apply_edits` / descriptions：无旧出参实现；负向测保留 `replace_all`/`expected_hash` 字符串。  
- `write` 的 `expected_hash` **保留**（非本包）。  
- `src/tool/fixtures/run_code_sdk.*`：**未动**（Tools 收口）。  
- `code_manifest.rs` 仍含旧 edit 教法句 → Tools 对齐时一并刷。

---

## 7. 状态

- 实现 + dig：**完成**  
- cargo：**未跑**（等 197 放槽 / mid 放行 198）  
- commit/push：**未做**

---

## 8. Acceptance 素材（供 mid 收口；未 commit）

### 8.1 done_when 对照

| brief 验收 | Runtime 状态 |
|---|---|
| Code 面仅 `edit` + `apply_edits` | ✅ companion 注入；关 edit 则二者皆无 |
| grep 无旧契约残留（实现面） | ✅ 负向/strip/write.expected_hash/历史 matrix 故意保留 |
| 失败跳过续跑；message 有指引 | ✅ `apply_edits` 串行 skip；多匹配/未找到进 `message` |
| cargo 绿 | ⏳ 未跑（等 mid 放行） |
| artifacts acceptance/diff-summary | ⏳ mid |
| push 实验分支 / 不合 main | ⏳ mid；本包不 commit |

### 8.2 Agent 视角最小场景（验收口述）

1. **单改再测：** `tools.edit({path, old_string, new_string})` → `ok===true` → `tools.run`/`wait` 看退出码（与 197 Command 同程序）。
2. **多匹配失败：** 同一 `old_string` 出现两次 → `ok===false`，`message` 提示加上下文；文件未改。
3. **批量跳过：** `apply_edits` 三项，中间项 miss → `results.length===3`，中间 `ok:false`，前后成功项已写入；不 throw `edit_failed`。
4. **删除：** `new_string: ""` → 成功 message 含删除确认。
5. **拒旧参：** 入参带 `replace_all` / `expected_hash` / 旧 `edits[]` on `edit` → `invalid_input`（或等价拒绝），无双出口。

### 8.3 建议 grep（收口验收）

```bash
# 实现/description 不应再教旧出参（负向测与 write.expected_hash 除外）
rg -n 'replace_all|expected_hash|"blocks"|\bapplied\b|edit_failed' \
  src/tools/edit.rs src/tools/edit/ src/tools/apply_edits.rs \
  src/tools/descriptions/edit.md src/tools/descriptions/apply_edits.md
```

### 8.4 放行后测命令（并行度≤2）

```bash
cd /workspace/litecode
export LC_MIN_MEM_GIB=2 CARGO_BUILD_JOBS=2
mkdir -p .litecode/ops/artifacts/WP-198
{
  echo "=== WP-198 edit triad $(date -Iseconds) ==="
  cargo test --lib -- tools::edit:: tools::apply_edits:: 2>&1
  echo EXIT_edit:$?
  echo "=== phase / matrix / registry companions ==="
  cargo test --lib -- phase2_chain_read_edit phase3_edit phase3_apply \
    contract_matrix code_gate_projects command_tools_have_no_settings 2>&1
  echo EXIT_phase:$?
} | tee .litecode/ops/artifacts/WP-198/cargo-edit-apply.txt
```

（`phase0_run_code_sdk_snapshot` 留给 Tools 刷完 197/198 共用 fixture 后再对齐。）

### 8.5 给 mid

- 实现 + dig + 本 acceptance 素材：**齐**  
- **不开 cargo** 直至你 priority「放行」  
- **不 commit**

## 9. cargo 修复

`contract_matrix`：`content carries facts` — `present_edit`/`present_apply_edits` 改为空 content（与 write 成功一致；指引只在 `data.message`）。

重跑后 **EXIT_edit:0**（34）、**EXIT_phase:0**（9，含 contract_matrix）。先不 commit。

## 10. 收口待命

mid：43 passed / 空 content 口径认；**Runtime 不 commit**；mid 统一收口 197+198 artifacts/commit/push/deliver。

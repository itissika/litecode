# WP-194 acceptance — done_when 对照

**Brief：** `.litecode/ops/artifacts/WP-194/brief.md`

| # | done_when | 结果 |
|---|-----------|------|
| 1 | 五工具描述 + SDK/PRELUDE 成功路径无过时 `data.*` 教法（历史对比可提旧信封但须标明废弃） | **通过**。`descriptions/{read,glob,grep,edit,write}.md` 与 `run_code_sdk.txt` 绑定长注释已改为裸 `file`/`lines`/`page`/…；全文检索无成功路径 `` `data.file` `` 等。PRELUDE 仍写「resolves to the tool's data」（指 resolve 值为 data 本体，非信封字段）。 |
| 2 | 行号/续页说法与 code 面一致 | **通过**。edit `old_string` 提示经 `FILE_LINE_PREFIX_HINT` 改为「勿粘贴带行号的外层账本/transcript」；PRELUDE 增「Pagination dialect: read → `start_line`，glob/grep → `cursor`」。 |
| 3 | 相关 fixture/测试绿；diff 证明未改执行核心 | **通过**。`sdk_snapshot` 2/2 ok；diff 仅描述/注释/PRELUDE/fixture/schema 字符串。 |
| 4 | 产物在 `artifacts/WP-194/`；合入实验分支（push 该分支，不要求合 main） | **本包交付**：`diff-summary.md` + `acceptance.md`；commit + push `feat/tool-contract-ptc`。 |

## 自检命令

```bash
rg '`data\.(file|lines|page|base|matches|created|blocks|applied|failed|searched|reason|paths)`' \
  src/tools/descriptions/{read,glob,grep,edit,write}.md src/tool/fixtures/run_code_sdk.txt
# 期望：无命中

LC_MIN_MEM_GIB=2 CARGO_BUILD_JOBS=1 cargo test --lib sdk_snapshot
```

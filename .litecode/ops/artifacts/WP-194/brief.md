# WP-194 实现 brief — PTC 文案对齐（agent 教法）

**分支：** `feat/tool-contract-ptc`（继续在实验分支上改，不推 main，除非 Andy 另说）  
**来源：** WP-193 建议优先两项。  
**口径：** 只改正文案/注释/PRELUDE，使描述与运行时「裸 resolve data」一致；**不改**工具运行时契约、权限、render、spill。

## 要做
1. **去掉 `data.` 信封腔：** 五工具描述（`src/tools/descriptions/{read,glob,grep,edit,write}.md`）与 SDK/绑定长注释（含 `src/tool/code_manifest.rs`、`src/tool/fixtures/run_code_sdk*` 等相关处）中，把 `data.file` / `data.lines` / `data.page` 等改为与运行时一致的裸字段名（`file` / `lines` / `page`…）。程序示例应写 `r.file` 而非 `r.data.file`。
2. **行号前缀说法：** 把「不要带 read 行号前缀」类 direct 残留改成「勿从带行号的外层文本/账本粘贴再 edit」；并（可选但建议）在 PRELUDE 加一句续页方言：read 用 `start_line`，glob/grep 用 `cursor`。
3. 同步任何会因文案字符串失败的 fixture/单测期望；改完自检描述与 SDK fixture 一致。

## 不做
- 不改 edit 半成功语义、不加强制查 `failed` 的运行时行为。
- 不改 Promise.all / `.status` 的硬拦截（属下一档产品取舍）。
- 不改人类侧 UI。

## 验收
1. 五工具描述 + SDK/PRELUDE 示例无过时的 `data.*` 成功路径教法（允许历史对比说明里提及旧信封，但须标明已废弃）。
2. 行号/续页说法与 code 面一致。
3. 相关 fixture/测试绿；diff 证明未改工具执行核心逻辑（或仅改注释字符串）。
4. 产物：`artifacts/WP-194/` 含 diff 摘要与验收说明；合入当前实验分支（push 该分支即可，不要求合 main）。

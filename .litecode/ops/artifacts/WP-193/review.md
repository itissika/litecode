# WP-193 审阅 — PTC / code 模式（agent 面）

**分支 / HEAD：** `feat/tool-contract-ptc` @ `bd71f8c`  
**口径：** 架构边界 · run_code 管线 · 文件工具设计；站在模型 agent 是否易理解、易用、易排错。  
**素材：** `brief.md`、`dig-runtime.md`（B+）、`dig-tools.md`（B+）、`dig-mid-notes.md`；对照 `docs/ptc/charter.md` 产品目标与 I1–I12。  
**性质：** 只读审阅；本包不改产品码。

---

## 总评

**档位：B+**

一句话：Code 面已经能让 agent 稳定工作——工具表只露出 `run_code`、错面调用有明确路由文案、程序侧「resolve 裸 data / reject ToolError / 不可捕获封口」与章程一致，外层结果按固定段落自渲染且不 spill；剩余问题集中在**文案仍带 `data.` 信封腔**、**edit 半成功易漏检**、以及**旧习惯（`.status` / `Promise.all`）**，会抬高试错成本，但**不是 blocker**，建议后续小包对齐描述与 SDK 注释，本包只交审阅。

---

## 大结论一：架构边界对 agent 清晰

产品目标是把「写一段程序、经 `tools.*` 编排、只把最终结果交回上下文」做成一等公民。现行实现按章程 D1/D2/D9 落地：五个文件工具声明 `Code`，不进 direct 列表；只要本 turn 至少有一个已准入的 Code 工具，就会自动挂上传输面 `run_code`，否则不出现。Agent 的体感很直接——工具表里没有独立的 `read`/`edit`，有 `run_code` 就意味着文件读写必须写进程序。

误用时的纠错路径也清楚。Direct 误调 Code 工具会看到类似 `` `read` is only available as tools.read inside run_code ``；Code 内误调 Direct 工具会被告知「是 direct、不可在 run_code 内用」。这与章程 I2 示例一致，系统提示里还有一句硬边界。一次失败通常就能学会。

仍有两处「少猜」不够彻底，但不挡主路径：准入失败时工具从两面消失、只打日志，agent 看不到「为什么少了某个工具」；无 host 时会把 code 工具落回 Direct（产品路径应不可达），若发生会与「勿直接调」的提示矛盾。整体上，**Direct vs Code 的边界对 agent 是清晰、可自愈的**。

---

## 大结论二：run_code 管线可预测、可纠错

从「决定写代码 → 执行 → 读回结果/错误」这条链，agent 看到的结构是稳定的。外层文本固定为：头行 → 失败段 → 错误明细 → diagnostics → 账本 → logs → result；硬顶约 64KB 分段裁剪并标注省略，且 `run_code` 作为传输面跳过 spill，大结果不会变成半截 `[blob:…]`。预算默认 120s（审批等待不计）、超时/取消/输出顶/内存栈不可 catch；封口后宽限取消在途调用，未结束则标 `unsettled` 并警示副作用可能已落地——与 I7/I10/I12 一致。

程序内契约同样对齐 remediation A1：成功 `await tools.x(...)` 拿到裸 data；领域失败 reject 可 catch 的 `ToolError`（稳定 `code`）；协议失败抛 `TypeError`；封口类失败不可捕获。已 catch 的 ToolError 会计入头行「rejected … handled by the program」，未 catch 则结束程序并取消在途调用。LSP 与路径风险只出现在外层 `diagnostics:`，不进 data——有意为之，但要求 agent **读完整次 `run_code` content**，不能只看 return value。

对纠错最伤的是习惯而非机制：历史会话里的 `.status` 信封会带偏模型（成功路径没有 `.status`）；`Promise.all` 批量写一旦有一次未捕获 reject，整段结束并可能留下 unsettled 副作用。PRELUDE 已要求 `allSettled` 与「无 status 信封」，但真实会话是否遵守仍需观察。空结果若带着 `reason: in_context_window` / `no_match` / `path_excluded`，也容易被当成「文件空」而改走 bash，绕过 code 账本。**管线本身可预测；坑在旧习惯与「只看 return、不看外层段落」。**

---

## 大结论三：五个文件工具——契约硬面好，教法有漂移

`read` / `glob` / `grep` / `edit` / `write` 在运行时已是 code 原生：裸 data、`FileRef`、`Page`、`reason`、稳定 ToolError 码、非法输入硬失败（无静默改写 pattern、无 suggested_*）、edit 部分成功用 `failed`/`blocks` 表达而非 `status:"partial"`。read→edit 主路径可写：用 `file.hash` 作 `expected_hash`，用无行号前缀的 `lines[].text` 拼 `old_string`；分页形状统一，但消费方言不同——read 的 `page.next` 是下一行号（作 `start_line`），glob/grep 的 `next` 是绑定查询的 cursor。

描述层与 SDK 长注释却仍大量写 `data.file` / `data.lines` / `data.page`。PRELUDE 和类型签名说的是「resolve 到工具的 data」，程序里应写 `r.file`；按描述写 `r.data.file` 会得到 `undefined`，最像「工具坏了」。edit 的「不要带 read 行号前缀」也是 direct 时代残留——code 面 `lines[].text` 本无前缀，却可能诱导模型从外层账本抄带行号文本再 edit（会硬失败 `read_line_prefix`）。另有：edit 半成功 resolve 后必须查 `failed`，否则文件已半改却 `return 'done'`；同一次 edit 内所有块对同一快照匹配，后块依赖前块 `new_string` 会 `chained` 失败，须拆成两次调用。

**结论：能力面可用、可链式；描述投影相对 A1 有漂移，抬高试错，不是硬 blocker。**

---

## 大结论四：风险、是否 blocker、后续建议

**非 blocker。** 正确写法已经写在 PRELUDE、类型签名与错面路由里；agent 按契约写可以完成文件编排。主要风险按伤害排序：

1. 描述/SDK 长文的 `data.*` 前缀与裸 resolve 不一致（高，文案对齐即可）。  
2. 旧 `.status` 习惯与 `Promise.all` 批量写（高，靠 PRELUDE/警示；是否加系统提示硬否定属产品取舍）。  
3. edit 半成功未查 `failed`、同 call 链式 edit（高/中高，描述已写，模型易漏）。  
4. `page.next` 双方言、空结果不读 `reason`、只看 return 漏掉外层 diagnostics（中）。  

建议后续小包（本包不改码）：把五工具描述与 SDK 绑定注释中的 `data.` 改为裸字段名；把「line-number prefix」改成「勿从带行号的外层文本粘贴」；可选在 PRELUDE 加一句「续页：read 用 `start_line`，glob/grep 用 `cursor`」。准入失败对 agent 不可见、host 缺失落回 Direct，属运维/产品路径问题，可另记观察，不必阻塞本审阅。

---

## 附录

### A. 关键证据路径（速查）

| 主题 | 路径 |
|------|------|
| 投影 / `run_code` 注入 | `src/tool/project.rs`；`src/tool/registry.rs`（`assemble_projected_tools`） |
| 错面路由文案 | `src/tool/executor.rs`（Direct↔Code 拒绝） |
| 系统提示边界 | `src/config/global_db/prompts/harness/tools.md` |
| PRELUDE / SDK | `src/tool/code_manifest.rs`；fixture `src/tool/fixtures/run_code_sdk.txt` |
| 外层 render / 64KB | `src/tools/code/render.rs` |
| spill 免除 | `src/tool/executor.rs`（`is_code_transport`）；`src/tools/code.rs` |
| resolve / ToolError 投影 | `src/tool/outcome.rs`（`program_result`）；`src/ptc/engine.rs` |
| 五工具描述 | `src/tools/descriptions/{read,glob,grep,edit,write}.md` |
| edit 半成功 / 快照规则 | `src/tools/edit.rs`；`src/tools/edit/planner.rs`（`SNAPSHOT_RULE`） |

### B. 对照章程 / remediation（agent 相关）

| 项 | 现状（agent 面） |
|----|------------------|
| D1 / D2 / D9 分面与 `run_code` 自动出现 | 已落地；五文件 Code，其余多 Direct |
| I2 错面路由提示 | 文案与示例一致，可自愈 |
| I7 取消封口 + unsettled | 已落地；宽限后警示重读 |
| I10 正交失败 + 64KB 视口 + 无 spill | 已落地 |
| I12 resolve data / ToolError / TypeError / 封口不可捕获 | 已落地；`retryable` 仅 conflict\|timeout |
| remediation A1 无 status 信封 | 运行时已无；描述仍写 `data.*` |
| remediation A3 edit 部分成功 | resolve + `failed`/`blocks`；无 `status:"partial"` |
| remediation A4 硬失败 / 不教 bash（code 路径） | 大体已落地 |

### C. 本包 dig 与 mid 摘录

| 文件 | 用途 |
|------|------|
| `.litecode/ops/artifacts/WP-193/dig-runtime.md` | Runtime：边界、管线、JS 契约、坑（总评 B+） |
| `.litecode/ops/artifacts/WP-193/dig-tools.md` | Tools：五文件描述/契约/链式/坑（总评 B+） |
| `.litecode/ops/artifacts/WP-193/dig-mid-notes.md` | 中层六项证据摘录（投影、PRELUDE、render、outcome、descriptions、harness） |
| `.litecode/ops/artifacts/WP-193/brief.md` | 本包 brief |

### D. 本包不做

不改 `src/**` / `docs/**`；不 cargo；不 commit / push。结论供 GM 审读后聊天发给 Andy，再 close。

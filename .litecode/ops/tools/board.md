# LiteCode Ops Tools Board

> 聚合面板：最近更新 · 使用教程 · 运行时版本感知
> 每次工具输出第一行均为版本头：`LITECODE_OPS tools_version=N tool=... tool_version=...`

## 当前版本

- `tools_version`: **7**
- registry: `.litecode/ops/tools/registry.json`
- 查询: `node .litecode/ops/tools/tools_version.mjs`

## 最近更新

| 时间 | 变更 |
|---|---|
| 2026-10-11 | v7：新增 ops.kb_check（知识库校验，复刻 litecode 解析，cases.json 87/87）；validate_all 接入 kb_check（有 error 即失败）；wp_gate 对知识类 WP 推到 review/done 前要求涉及节点无 error（工具集管家）|
| 2026-10-11 | v6：wp_gate 通过后写回 board.json 状态 + 自动 append status_change log + 重跑 validate（失败回滚）；--dry-run 保留只检查（工具集管家）|
| 2026-10-11 | v5：工具一览补 emit/ack；ops.board 版本对齐；emit/ack 补 stdout_header（工具集管家）|
| 2026-10-11 | v4：事件总线 emit/ack；工具维护权移交工具集管家；GM 仅 tick 兜底
| 2026-10-11 | v3：stdout 首行版本头；本 board；team-ops bot 已建且未发首条 |
| 2026-10-11 | v2：validate 允许人类审查员 andy |
| 2026-10-11 | v1：gate / tick / validate / append_log |

## 使用教程（强制）

1. 调用前感知版本：读输出首行或跑 `tools_version.mjs`
2. 推进 WP：`wp_gate.mjs --id WP-xxx --to <status>`（通过即写回 board.json 并记 log；只检查加 `--dry-run`）；done 需产物 + tool_traces
3. 审计：`append_log.mjs --kind --source --summary [--wp]`
4. 每轮：`tick.mjs`
5. 热更：改完 `tools_version++`，并在本表追加一行；破坏性变更用新 tool id
6. 知识库自查（v7）：改/建知识节点后跑 `kb_check.mjs`（只看一个节点加 `--node <key>`），error 必须清零；`validate_all` 会同样拦截。详见下方「ops.kb_check 用法」

## 工具一览

| id | 命令 |
|---|---|
| ops.tools_version | `node .litecode/ops/tools/tools_version.mjs` |
| ops.validate_board | `node .litecode/ops/tools/validate_all.mjs [--wp WP-xxx]` |
| ops.append_log | `node .litecode/ops/tools/append_log.mjs --kind --source --summary [--evidence] [--wp]` |
| ops.wp_gate | `node .litecode/ops/tools/wp_gate.mjs --id WP-xxx --to <status> [--dry-run] [--source <agent_id>] [--kb-nodes k1,k2]` |
| ops.tick | `node .litecode/ops/tools/tick.mjs [--wp WP-xxx]` |
| ops.board | Read `.litecode/ops/tools/board.md` |
| ops.emit_event | `node .litecode/ops/tools/emit_event.mjs --type --source --summary [--wp]` |
| ops.ack_event | `node .litecode/ops/tools/ack_event.mjs --id E-... --by <agent_id> [--wp]` |
| ops.kb_check | `node .litecode/ops/tools/kb_check.mjs [--node <key>] [--json] [--wp WP-xxx] [--stock-ref <rev>] [--selftest]` |

## ops.kb_check 用法（v7）

知识库校验。解析部分是 litecode 的 Node 复刻（`kb_parse.mjs` / `kb_scopes.mjs`，对应 `src/knowledge/{document,mentions,validate,fsref,index}.rs` 与 `src/engines/code_search/enclosing.rs`），以 `tests/fixtures/knowledge_refs/cases.json` 为准：`--selftest`（= `node .litecode/ops/tools/kb_check.test.mjs`）必须全过才算可用。不改 litecode 本体、不加依赖。

| 参数 | 作用 |
|---|---|
| （无） | 检查 `knowledge/` 全部节点，打印每条问题 + 末行 JSON 摘要 |
| `--node <key>` | 只报该节点的问题（键或 `knowledge/…md` 路径）；仍按全库解析（悬空/重复要看全库） |
| `--json` | 首行版本头之后输出一个 JSON：`{ok, nodes, errors, warnings, by_code, findings[]}` |
| `--wp WP-xxx` | 写 tool_traces |
| `--stock-ref <rev>` | 存量基线（默认取准入文档「基线提交」`e583ff2`）：该提交里已有的节点算存量，其余算新节点 |
| `--selftest` | 跑 cases.json 对照测试 |

退出码：0 = 无 error（可有 warning）；1 = 有 error；2 = 参数错误 / `--node` 找不到。

**error 与 warning**：error 必须修，`validate_all` 失败、知识类 WP 过不了 `wp_gate`；warning 只提示。新节点不满足准入规则 → error；存量节点同样问题 → 仅 warning。

| code | 级别 | 含义 |
|---|---|---|
| `ref_malformed` | error | 看起来像引用、但 litecode 会静默当普通文本（`[@key=`、`[ @`、全角括号/引号、未知属性、顺序不是 file→symbol→lines、key 不合规、file 为空、缺 `]`、`=` 两侧空格、key 与 file 混用…；`args.reason` 给出原因）。行内代码里的写法不算 |
| `ref_legacy` | error | 旧写法 `[[node : K]]`（KR-SYN-7） |
| `filename_mismatch` | error | 文件名主干 ≠ 键（litecode 为 warning；准入 §0「必须」，这里升为 error） |
| `dangling_relation` / `self_relation` | error | `[@ key]` 目标不存在 / 指向自己 |
| `missing_file` / `symlink_escape` / `unreadable_file` | error | 文件不存在（逐段精确匹配、区分大小写）/ 链接出工作区 / 不可读 |
| `missing_symbol` / `ambiguous_symbol` / `unsupported_language` | error | 符号链找不到 / 不唯一 / 语言不支持 |
| `invalid_lines` / `line_out_of_range` | error | 行号格式错（含符号引用带 lines）/ 越界 |
| `invalid_status` / `empty_key` / `duplicate_key` | error | 声明层问题 |
| `inactive_target` | warning | enabled 节点引用 pending/disabled 节点 |
| `symbol_drift` | warning | 符号正文与 HEAD 不同（准入 §6：视为证据失效，提交前重新核对） |
| `symbol_unverified` | warning | Go/C/C++/Java/C#/JSX/TSX 只做近似符号解析，找不到/不唯一时降为警告，需 litecode 索引确认 |
| `domain_missing` / `domain_unknown` | 新 error · 存量 warning | 声明块缺 `domain : <域 id>` / 域不在 board.json（team-ops 不产出节点） |
| `parent_missing` / `parent_multiple` | 新 error · 存量 warning | `上层：` / `父节点：` 二选一且恰好引用一个节点 |
| `baseline_missing` | 新 error · 存量 warning | 正文缺 `基线：<短哈希>` |
| `tag_unknown` | 新 error · 存量 warning | 「标签：[@ …]」不在五个里：上层 / 父节点 / 契约展开·框架展开·向下展开 / 相关 / 依据（模板 §7.1 的 `证据：` 视同 依据） |
| `new_not_pending` | error | 新节点必须 `status : pending` |
| `dup_primary_suspect` | warning | 两个节点引用同一符号/行范围作证据，或 summary 完全相同 → 疑似主节点重复 |
| `ref_in_declaration` | warning | 声明块里的 `[@ …]` 不是引用 |
| `not_a_node` | warning | 知识根下的 `.md` 没有声明块，不是节点 |

接入：`validate_all` 每次都跑全库，error > 0 即失败；`wp_gate --to review|done` 对知识类 WP（domain=knowledge，或 WP 文本里写了 `knowledge/…md` / `[@ key="…"]` / `node:<key>`，或有节点 `domain :` 等于该 WP 的域，或传 `--kb-nodes`）要求涉及节点无 error，`--dry-run` 同样检查。基线结果：`.litecode/ops/artifacts/kb_check-baseline.md`。

## 待发掘

- kb_check：Go / C / C++ / Java / C# / JSX / TSX 的符号解析是近似实现（仓库目前没有此类符号引用；Rust/TS/JS/Python 已与 litecode 的 tree-sitter 结果逐文件核对一致）
- kb_check：漂移（symbol_drift）用 git CLI 读 HEAD，litecode 用 git2；快照仓库判定（git_dir_is_snapshot）未复刻
- 使用中发现的其它缺项记在这里，由工具集管家热更

## 边界

- 仅 team-ops 维护；产品 bot / 群对本启动无感知，直至 Andy 群内统一启动
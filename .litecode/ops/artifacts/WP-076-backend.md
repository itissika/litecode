# WP-076-backend · 后端中层方案（WP-077）

- 角色：backend_mid；父 WP-076（gm，Andy 授权，in_progress）；本包 WP-077（split，approver=gm）；前端兄弟包 WP-078（frontend_mid）。
- 基线：分支 `ops/team-framework`；中层只拆活 / 交叉审 / 汇总；**不写 `src/`、`tests/`**；**不 commit**；不碰 `.github/workflows/windows-*.yml`。
- 时间：2026-10-13（HKT）。产物：本文件（deliver 时覆写为汇总）。
- 依据：IDE / Tools / Session / 前端中层只读调查（10-13 12:20 后回报）+ GM 裁定。

## 1. Goal

1. **检索**：代码语料文件改动后后台静默增量刷新（30s），重建只在首建或不兼容；代码/会话索引计划拆分。手动 Refresh index 保留为强制刷新。
2. **ask_user 方案 B**：free text 不再由 agent 控制；每题系统固定末项「其他（自己填写）」；单选互斥、多选并存、只填文字算答完；wire/答案/重放契约不变。

## 2. GM 裁定（锁定口径）

| 项 | 裁定 |
|---|---|
| 检索 (a) | 30s 周期，与会话定时器 `router.rs:32 SESSION_INDEX_TICK` / `:178-209` 同构；**不另造去抖** |
| 检索 (b) | 不避开回合（刷新期间读索引不等 worker，`code_search_engine.rs:176`） |
| 检索 (c) | 首建提示不在本包 |
| 检索 (d) | CLI 不在本包 → **ISSUE-050**（ide-substrate，产品未定） |
| 检索 ①② | 周期刷新 + 代码/会话计划拆分都做 |
| ask (a) | 末项为行内输入框 |
| ask (b) | 单选：自由输入与选项互斥（打字即选中末项）；多选：可并存 |
| ask (c) | 只填文字算答完 |
| ask (d) | 文案「其他（自己填写）」，前端 / CLI 一致 |
| ask (e) | agent 自加 Other 只靠描述劝阻，不拒绝 |

## 3. 子包树

```
WP-076 (gm, in_progress)
  ├── WP-077 (backend_mid, approver=gm)  ← 本包
  │     ├── WP-079 owner=ide      ide-substrate     检索周期刷新 + 计划拆分
  │     ├── WP-080 owner=tools    tools-permission  ask_user schema/描述/format_answers
  │     └── WP-081 owner=session  harness-core      AskQuestion 恒 true + CLI 末项 + 核查
  └── WP-078 (frontend_mid, approver=gm)  AskUserCard 末项行内输入
```

| 子包 | artifact |
|---|---|
| WP-079 | `.litecode/ops/artifacts/WP-076-ide.md` |
| WP-080 | `.litecode/ops/artifacts/WP-076-tools.md` |
| WP-081 | `.litecode/ops/artifacts/WP-076-session.md` |

## 4. 串并行与文件竞态

| 包 | 主文件 | 并行 | 依赖 |
|---|---|---|---|
| WP-079 IDE | `src/serve/router.rs`、`src/engines/mod.rs`（engine_index_work :204、spawn_scoped_refresh ~:937）、`src/engines/code_search/**`；集成测 | 与 080/081 全并行 | 无 |
| WP-080 Tools | `src/tools/ask_user.rs`、`descriptions/ask_user.md` | 与 079/081 并行 | 无 |
| WP-081 Session | `src/.../grants.rs`（AskQuestion）、`src/cli.rs:206-237`、核查 `plan.rs`、`protocol.rs` | 与 079/080 并行 | 与 080 共享「恒 free_text=true」口径；`grants.rs` 只归 Session，`ask_user.rs` 只归 Tools，互不越界 |

注意：`cli.rs` 有 Session（:206-237 ask 交互）与 ISSUE-050（:661）两处，本包只有 Session 动 :206-237；IDE 不动 cli.rs。

## 5. 域口径

### WP-079 · IDE
- router 会话定时器旁加代码语料周期 tick（30s，同构，不加去抖）。触发条件全满足：engine desired、状态 Warm、不在刷新中、待办**仅增量**；满足才 `consume_code_index_work`。待办为重建 → 不触发（只能首建 / 不兼容 / 手动）。
- 拆分 `engine_index_work`：代码与会话各自计划、各自执行；会话侧需重建时，手动 refresh 不得把 code_search 带入 rebuild / Warming（修调查 ⑤ 疑点，预估常见 bug）。
- 不改前端、client_protocol、cli.rs。
- 验收（GM 锁定四条）：
  1. 集成测：hash 嵌入、引擎 Warm，写文件**不调 refresh**，限时内断言 pending→0 且新内容可搜。
  2. 单测：代码仅增量 + 会话需重建时，手动 refresh 不置 code_search Warming。
  3. 单测：待办为重建时自动刷新不触发。
  4. death_list：后台路径不得调 `begin_building`，非首建/不兼容不得置 Warming。

### WP-080 · Tools
- `ask_user.rs`：删 schema `allow_free_text`（:82-85、:112-115）与解析别名 `free_text`（:223-227）；旧 agent 传入**静默忽略**；构造 AskQuestion 时恒 `free_text=true`。
- `descriptions/ask_user.md:3-4`：写明系统会为每题自动加末项「其他（自己填写）」，agent 不要自己加 Other / 其他（劝阻，不拒绝）。
- `format_answers`（:313-350）：`selected` 为空 + free_text 非空 → 正确输出（selected: (none) + free_text）；补测。
- 不引入保留 option id（方案 A 已否）；不改 wire。

### WP-081 · Session
- `grants.rs:65 AskQuestion.free_text` 保留、恒 true（兼容旧客户端）；wire 字段（protocol.rs:493-513）、`AskAnswer{selected,free_text?}`、`agent/permission` 回包、重放契约**零变化**（快照不变即证据）。
- CLI `cli.rs:206-237`：每题末项「其他（自己填写）」；单选与选项互斥；多选可与选项并存；只填文字算答完；补测。
- 核查并写进产物：`plan.rs` 对 free_text 的用法；`permission_request` 是否落盘 / 重连重建（若重建，确认恒 true 生效于重建路径）。

## 6. 跨域协议对齐（WP-077 ↔ WP-078）

由后端中层与前端中层共同对齐，写进双方产物：

| 契约点 | 后端（WP-080/081） | 前端（WP-078） |
|---|---|---|
| `AskQuestion.free_text` | 恒 true，字段保留 | 忽略该字段，每题固定渲染末项 |
| 末项文案 | CLI「其他（自己填写）」 | 同文案，行内输入框为最后一项 |
| 单选 | CLI 选末项=互斥 | 打字即选中末项，取消其它选项 |
| 多选 | 可并存 | 可并存 |
| 完成判定 | 选了或 free_text 非空 | `allAnswered` 放宽为同条件；`buildAnswers` 去 `q.free_text` 条件 |
| 回包 | `answers: map<qid, AskAnswer{selected, free_text?}>` 不变；末项**不**进 `selected` | 同：不往 selected 塞假选项 |
| 空白 | 去首尾空白，空串当 None（grants.rs 现有） | 发前可 trim，后端兜底 |
| 不动 | 顶层 free_text（审批卡）、AskCard/PermissionModal | 同 |

改动顺序无强依赖：后端恒 true 后旧前端仍显示输入框；新前端忽略字段亦兼容旧后端。

## 7. 禁区

- 中层不写 `src/` / `tests/`；域包不越域。
- 不改 wire 协议；不加保留 option id。
- 检索：后台路径不重建；不做首建提示；不改 CLI 检索（ISSUE-050）。
- 不 commit / push；不碰 `.github/workflows/windows-*.yml`。
- optional_lsp 若需跑一律 `</dev/null`。
- LSP 通用规则（Andy 10-13）本包不涉及 LSP。

## 8. 验收清单（WP-077 deliver 前）

- [ ] WP-079/080/081 done（或明确 blocked）
- [ ] §5 各域验收测绿，中层抽样复跑
- [ ] §6 对齐表与 WP-078 产物一致（交叉审）
- [ ] ISSUE-050 已立（open，产品未定）
- [ ] `validate_all.mjs` ok:true；未 commit

## 9. 请 GM

方案已填实，子包 WP-079/080/081 均 drafting。请 `approve --id WP-077`；批后中层再 start，并批域包 plan 后开工。

## §7 交付汇总（2026-10-13 12:5x，后端中层）
- WP-079 IDE：done。engines `code_index_tick()`（desired+Warm+不忙+增量才刷新；重建/无待办跳过；不 begin_building、不置 Warming），代码/会话计划拆分。产物 WP-076-ide.md。
- WP-081 Session：done。grants 恒 free_text=true；CLI 末项「其他（自己填写）」，单选互斥/多选并存/仅文字算答完；router.rs `CODE_INDEX_TICK=30s` 循环调 `code_index_tick`（补充）。重连不可重建为既有行为，未做。
- WP-080 Tools：done。schema 去 allow_free_text（旧字段静默忽略、恒 true）；描述引用「其他（自己填写）」只劝阻；format_answers 支持仅文字。
- 中层复验：lib engines:: 连跑 5 次全绿（276）；optional_code_search 19/19；code_search_ipc 11/11；code_index_tick_death_list 2/2；tools::ask_user 9/9；cli 相关 50/50；cargo fmt --check 干净；validate_all ok。
- Issue：新开 ISSUE-050（CLI 无后台刷新，产品未定）、ISSUE-051（Tools maybe_consume_index 待办为重建时也触发）、ISSUE-052（两条旧测试口径漂移，已修，HEAD 466acad 基线即失败）。
- 前端依赖：WP-078 需放宽 AskUserCard allAnswered 为「选了或文字非空」，已告知前端中层。
- 未 commit；windows-*.yml 未动。

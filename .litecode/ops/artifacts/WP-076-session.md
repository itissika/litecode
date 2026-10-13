# WP-081 方案（Session）：ask_user 协议恒 free_text + CLI 末项「其他（自己填写）」
产物：.litecode/ops/artifacts/WP-076-session.md；approver=backend_mid；批后再 start；不 commit；ask_user.rs 归 Tools(WP-080)，本包不碰。

## 1 后端（grants.rs，Session 域）
- grants.rs:57-66 `AskQuestion.free_text` 字段保留（serde default 不变，兼容旧客户端）；语义注释改为「ask_user 恒 true」。
- grants.rs:147-157 `effective_questions` 旧单题 q0 提升路径：`free_text: self.free_text` → 恒 `true`（AskKind::AskUser 分支）。
- grants.rs:126-142 `AskPrompt::ask_user` 顶层 free_text 保持 false（顶层仅审批卡用），不动。
- 解析侧「忽略 agent 的 allow_free_text、构造时恒 true、删 schema/描述」由 Tools WP-080 在 ask_user.rs:82-85/:112-115/:223-227/:253-262 与 descriptions/ask_user.md:3-4 落地；Session 对齐口径并交叉核。
- wire（project.rs:441-460 agent/permission_request；protocol.rs:493-513）、AskAnswer{selected,free_text?}（grants.rs:70-75）、回包、重放契约零变化。

## 2 CLI（src/cli/cli.rs:198-258 ask_cli_one_question）
- 每题选项列表末尾追加 `[N+1] 其他（自己填写）`，不依赖 q.free_text。
- 单选：选末项 → 提示输入文字，selected=[]、free_text=Some；选普通项 → 不再追问文字（互斥）。
- 多选：可同时含普通项与末项；含末项则追问文字；末项编号不进 selected。
- 判定：selected 非空 或 free_text 非空 即答完；选了末项但文字为空 → 视为该题未答（若也无其他选中）。s/skip 行为不变。
- 输出仍为 AskAnswer{selected, free_text}；抽纯函数 `parse_cli_selection(input, q) -> (Vec<String>, bool wants_text)` 以便单测（不需 stdin）。

## 3 测试
- grants.rs：effective_questions 旧 q0 路径 free_text==true；AskQuestion 反序列化缺字段/false 后经 ask_user 构造仍恒 true（与 WP-080 测试互补）。
- cli.rs：parse_cli_selection 单选末项→selected 空+要文字；单选普通项→不要文字；多选 "1,3(末项)" 共存；越界/空输入；仅文字答完。
- 协议快照（permission_request_serializes_request_id 等）不变即证据。
- format_answers selected 空+free_text（ask_user.rs:313-350）由 WP-080 补；Session 交叉核。
- 跑：PATH=/home/box/.cargo/bin:$PATH cargo test 仅 permission / cli / client_protocol 相关。

## 4 核查结论（只读）
- plan.rs：只用 `AskPrompt::approval`（plan.rs:205，grants.rs:113-123 顶层 free_text=true，questions 空）与 `reply.free_text` 作为「User opinion」（plan.rs:214、:253）；不经 AskQuestion，本改动不影响。
- 落盘/重连：permission_request 不落盘。pending 仅在连接任务局部内存 `pending_asks: HashMap`（connection.rs:1535，插入 :1623-1632）；PendingAsk 只存 session_id/agent_name/tool/rule_id/reply_tx，不含 questions，无法重建卡片；断连/取消/退出走 reject_all_pending_asks（connection.rs:1445-1458）回 Deny。stray_grants（:1485-1498、:1728-1733）只缓冲早到/错配的 grant，不是 ask 重放。前端仅实时消费 agent/permission_request（web/src/stores/connectionStore.ts:401-403 → turnStore.ts:702），无 rehydrate。⇒ 不存在「重建路径」，恒 true 只需在 ask_user 构造 + effective_questions 生效。
- 风险：①旧前端仍按 q.free_text 渲染，恒 true 后即显示文字框，兼容；②旧前端「必须先选一项」限制需前端放宽，否则仅文字答不完；③未来若做 ask 持久化重建，需带 questions 且保证恒 true。

## 5 前端接口（非本域，前端中层）
- 忽略 q.free_text，每题固定末项「其他（自己填写）」；allAnswered = selected 非空 或 free_text 非空；buildAnswers 去 q.free_text 条件，末项不进 selected。顶层 free_text（审批卡）不动。

## WP-081 落地（Session · 2026-10-13）

### Diff 摘要（未 commit）
- `src/permission/grants.rs:147-161` `effective_questions` legacy q0 路径 `free_text` 恒为 `true`（:158）；顶层 approval `free_text`（:121）未动；`AskQuestion.free_text` 字段保留。wire/answer/replay 契约不变。
- `src/permission/grants.rs:340` 新测 `wp081_legacy_q0_free_text_always_true`。
- `src/cli/cli.rs:205-308`：新增 `CLI_OTHER_LABEL`、纯函数 `parse_cli_selection`(:213)、`build_cli_answer`(:250)；`ask_cli_one_question`(:276) 无视 `q.free_text`，恒追加末项「其他（自己填写）」。单选互斥（首个有效项生效）；多选可并存；只填文字算答完；选「其他」但文字空 = 未答；「其他」不进 `selected`。输出仍为 `AskAnswer{selected, free_text}`。
- `src/cli/cli.rs:311` 新测模块 `wp081_cli_tests`（单选选末项/多选并存/只填文字与空文字/越界与空输入）。
- 未动 `src/tools/ask_user.rs`、descriptions（WP-080 所有）。

### 测试
- `cargo test --lib -- wp081 permission::grants`：8 passed / 0 failed。
- `cargo test --lib -- ask_user permission`：32 passed / 0 failed。
- `cargo test --test client_protocol_permission`（含 `permission_request_serializes_request_id` 完整 wire 快照）：7 passed / 0 failed；`tests/` 无改动 → 协议快照不变。

### 范围外记录
- `permission_request` 只存在于连接内存、不落盘，重连后无法重建（按拒绝处理）：既有行为，本包不做。

### 交叉审（Session · WP-080）
WP-080 仍 in_progress，未找到 Tools 产物文件（无 WP-080 / WP-076-tools 产物），故写于此处。代码已在工作区（只读核对）：
- `src/tools/ask_user.rs:218`、`:244` 忽略 `allow_free_text`/`free_text`，恒 `true` ✓
- schema 两处 `allow_free_text` 已删 ✓；`descriptions/ask_user.md` 已改为系统自动追加「其他（自己填写）」✓
- `format_answers` :301-316 处理 selected 空 + 文字 ✓
- 测试 :476 `schema_has_no_allow_free_text`、`legacy_free_text_flags_ignored_free_text_always_true`、:512 `text_only_answer_is_formatted`；`cargo test --lib -- tools::ask_user` 9/0 ✓
- 与 Session 侧一致：答案结构 `AskAnswer{selected:[], free_text}` 与 CLI/grants 口径相同。
- 结论：**有条件接受**。条件（非阻断）：Tools 需补产物文件并 deliver；`rustfmt --check src/tools/ask_user.rs` 有 import 排序差异，请收口前 fmt。

## router code_index_tick（WP-077 补充）

- 接口核实：`src/engines/mod.rs:930 pub fn code_index_tick(&self) -> CodeIndexTick`（sync、非阻塞），未改 src/engines。
- diff（仅 `src/serve/router.rs`，+15 行）：
  - :33-35 新增 `const CODE_INDEX_TICK = 30s`（紧随 `SESSION_INDEX_TICK`）
  - :214-225 会话 tick 块之后新增同构 spawn 块：`tokio::time::interval(CODE_INDEX_TICK)` + `MissedTickBehavior::Delay`，循环 `let _ = engines.code_index_tick();`；不按 turn 门控（接口声明 safe during turns），无日志；关停与会话 tick 一致（随 runtime 结束，无显式取消）。
- CLI 未接（ISSUE-050）。未 commit。
- 验证：`cargo build` 通过（仅既有 warning）；router.rs 无 fmt 差异（`cargo fmt --check` 仅报 Runtime/引擎侧 src/engines/mod.rs:1780 与 tests/code_index_tick_death_list.rs:50，非本改动、禁区未动）；`cargo test serve` 全部 0 failed（lib 66 passed，其余集成测按过滤通过）。

# WP-078 方案（frontend_mid · split）

父包：WP-076（GM，approver andy）· 并行：WP-077（backend_mid）
审批：gm · 产物：本文件 + 子包产物 `WP-076-web.md`
中层只拆活 / 交叉审 / 汇总；**未经 GM 批 plan 不 start；不 commit**。

## Goal
ask_user 方案 B：agent 只定题数 / 单多选 / 选项；系统固定每题**末项**为行内自由输入，label「其他（自己填写）」。

## GM 裁定（落地口径）
1. 末项 = 行内输入框（选项列表最后一行），**每题都有**，不可关闭。
2. 单选：与选项**互斥**——输入（trim 后非空）即选中末项，并**清空**已选选项；点选某选项则末项失去选中（文本可保留但不提交）。
3. 多选：末项可与选项**共存**。
4. 仅有文本（trim 后非空）也算已答（单选、多选都算）。
5. agent 自加 "Other" 选项仅靠 description 劝阻，前端不做去重。

## 已确认协议（backend_mid 已确认 5 点 + 补充）
- agent schema 删除 `allow_free_text` / `free_text`（questions[] 内及旧式顶层）；旧调用带了**容忍并忽略**。
- `AskQuestion.free_text`（grants.rs）保留，**写死 true**；前端**忽略**此字段。
- 顶层 `PermissionRequest.free_text`（审批卡用）**不动**。
- `AskAnswer { selected, free_text? }` 结构与回放格式**不变**。
- `selected` 可为空、只带 `free_text` = 已答：单选**和多选**都成立；backend format_answers（ask_user.rs:323-342）负责接受。
- 单选打字时：末项选中，`selected` 置空，提交时 `selected: []` + `free_text`。
- 末项**不是**协议 option，**没有** option id，不进 `options[]`、不进 `selected`。
- `free_text` 按 trim 后判空：前端判断"已答"和提交时都 trim；trim 后为空 = 未填，不发送该字段。
- 更新 `src/tools/descriptions/ask_user.md`（含劝阻自加 Other）。

## 后端依赖（占位）
- WP-077（backend_mid）下的子包，**编号待 backend_mid 给出**：
  - Tools：`src/tools/ask_user.rs` schema 删字段 + 忽略旧字段 + 答案接受 `selected` 为空；`ask_user.md`。
  - Session：`src/permission/grants.rs` `free_text` 写死 true；CLI 交互同口径。
- 前端子包可与后端并行开发（前端忽略 `free_text`，不依赖后端先落）；**联调验收**需后端子包完成。

## 子包
| 子包 | owner | 域 | 产物 |
|------|-------|----|------|
| **WP-082** | `frontend`（前端-web） | client-ux-web | `WP-076-web.md` |

## web 落点（域 bot 实施）
- `web/src/components/AskUserCard.tsx`
  - 删 `freeText` prop（:36,44,56,58）；去掉 `active?.free_text` 门控（:221）与 `q.free_text` 门控（:144）。
  - 输入框从 actions 区挪到每题选项列表**末行**，label「其他（自己填写）」，带 testid `ask-user-free-text-${qid}`。
  - 单选互斥：输入非空 → 清空 `pickedByQ[qid]`；点选项 → 末项不再算选中（提交不带 free_text）。单选"点选即跳题"不受打字影响（打字不跳题）。
  - 多选共存。
  - `allAnswered`：`selected.length > 0 || trim(text) !== ""`。
  - `buildAnswers`：trim；单选时末项选中则 `selected: []`。
- `web/src/lib/askUser.ts`：`normalizeAskQuestions` 删 `freeText` 参数，不再写 `free_text`。
- `web/src/stores/turnStore.ts`：:712 不再传 `pr.free_text` 给 normalize；`PendingPermission.free_text`（:45,:721）保留给审批卡。
- `web/src/api/types.ts`：`AskQuestion.free_text` 注释改为"后端写死 true，前端忽略（deprecated）"；顶层注释改为仅审批卡用。
- 测试（新增/更新；`turnStore.test.ts`、`ComposerDock.test.tsx`、新增或扩充 AskUserCard 测试）：
  1. 每题都有末项输入（含 `free_text:false` / 缺字段的 payload）；
  2. label「其他（自己填写）」；
  3. 单选互斥：先选项后打字 → selected 清空；先打字后选项 → 不带 free_text；
  4. 多选共存：selected + free_text 同时提交；
  5. 只有文本 = 已答（单选、多选），Submit 可用；
  6. 纯空白 = 未答，不发 free_text。

## desktop
desktop 内嵌 web，预计**无改动**；web 子包验收时在 desktop dev 壳里手点一次 ask_user 卡核对（中层验证，不另开子包）。

## 不在范围
- 审批卡 freeText：`web/src/components/AskCard.tsx`、`PermissionModal.tsx`、顶层 `free_text`——**不动**。
- 后端代码（WP-077）；语义检索（WP-076 A 部分，归后端）；不 commit。

## 验收标准
- 上面 6 项测试通过；`web` 的 typecheck / lint / 单测全绿。
- 渲染与提交忽略 `AskQuestion.free_text`；审批卡行为与测试不变。
- 与后端联调：单选只填文本 → agent 收到 `[qid] free_text: …` 且无 selected 报错；多选共存正常。
- desktop 手点核对通过；禁区未碰；未 commit。

## 分级
- 1（产品待定，已由 GM 裁定）：末项形态 / 互斥 / 仅文本算答 → 已定。
- 5（边缘）：现状输入框跟着当前题走、放在操作区，切题时易误会——本包随改动一并消除。

## 交付小结（frontend_mid 交叉审，2026-10-13）
- WP-082 已 close done。diff 范围：AskUserCard.tsx / askUser.ts / turnStore.ts / types.ts（仅注释）/ 测试（新增 AskUserCard.test.tsx，更新 PermissionModal.test.tsx 中 ask_user 断言、turnStore.test.ts）。AskCard.tsx / PermissionModal.tsx 未动；`freeText` 保留为可选、忽略的 prop（PermissionModal 仍传）——接受。
- 口径核对：末项「其他（自己填写）」每题都有、无 option id；单选打字清空已选、提交 selected []；点选后文本不提交；多选共存；仅文本即已答；trim 后空 = 未答且不发 free_text。
- 中层复跑（/exec-daemon/node v22.14.0）：vitest AskUserCard/turnStore/PermissionModal → 3 files / 71 passed；`tsc -b` → 0。lint：N/A（web 未配置 lint 工具，不阻塞）。web 自报全量 1239 passed。
- desktop 内嵌 web，壳无需改动。
- 后续（不阻塞本包）：WP-080/081 后端落地后做 e2e 联调 + desktop 手点核对。
- 未 commit。

## WP-083 + 前后端 e2e 核对（frontend_mid，2026-10-13）
- WP-083 close done：末项常驻可见 label「其他（自己填写）」（input 经 aria-labelledby 取名）；选中态与选项同用 `btn-primary`/`btn-ghost`（AskUserCard.tsx:296 选项 / :312 末项），测试有 classList 断言（AskUserCard.test.tsx:142-147）。复跑（node v22.14.0）：3 files / 76 passed；`tsc -b` 0；lint N/A。
- 后端（WP-080/081）：`cargo test --lib ask_user` 9 passed（含 schema_has_no_allow_free_text、legacy_free_text_flags_ignored_free_text_always_true、text_only_answer_is_formatted）；`cargo test --lib permission::grants` 4 passed（含 wp081_legacy_q0_free_text_always_true）。schema 全文不含 allow_free_text（ask_user.rs:476-483）；questions[] 与旧式顶层 free_text 写死 true（ask_user.rs:218,244）。
- format_answers（ask_user.rs:301-339）：selected 为空且有 free_text → `[qid] selected: (none) — answered via 「其他（自己填写）」` + `[qid] free_text: …`；grants.rs:194-197 对 free_text 做 trim_opt。
- 前端判答 = 已选 > 0 或 trim 后文字非空，单选、多选同一规则；提交 `{selected, free_text?}`：单选只填文字 → `selected: []`；末项不进 selected、没有 option id；trim 后为空不发 free_text。与后端期望一致。CLI 同样用「其他（自己填写）」（cli.rs:200-212）。
- desktop：静态核对（代码层面，箱内没做 GUI 手点）。desktop 用 `safeLoadURL` 加载 sidecar 提供的 workbench http URL（desktop/src/main.ts:293-307，sidecar 目录含 web/dist：paths.ts:5），desktop/src 里没有任何 ask_user / AskUser 渲染代码 → 跑的是同一套 web 构建，壳不用改。
- WP-078 按 GM 指示暂不交付，等 WP-079 后端落地后再交。

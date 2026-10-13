# WP-082 · AskUserCard 末项固定自由输入（方案B）· 实施记录

父包 WP-078（口径 WP-076-frontend.md）· owner frontend（前端-web）· approver frontend_mid · 未 commit

## 改动
- `web/src/components/AskUserCard.tsx`：每题选项列表末行固定输入框（`OTHER_LABEL`=「其他（自己填写）」，testid `ask-user-free-text-${qid}`，无 option id）；不再看 `free_text`。单选：打字(trim 非空)清空已选，提交 `selected: []`+free_text；点选项后文本不提交。多选共存。`allAnswered` = 已选 > 0 或 trim 后有文本；buildAnswers trim，空不发 free_text。`freeText` prop 改为可选且忽略（PermissionModal.tsx 仍传该 prop，按禁区未改）。
- `web/src/lib/askUser.ts`：`normalizeAskQuestions` 删 freeText 参数，不写 free_text。
- `web/src/stores/turnStore.ts`：不再传 `pr.free_text` 给 normalize；`PendingPermission.free_text` 保留给审批卡。
- `web/src/api/types.ts`：仅改注释（AskQuestion.free_text deprecated、前端忽略；顶层仅审批卡）。
- 测试：新增 `web/src/components/AskUserCard.test.tsx`（6 项口径，7 个用例：单选互斥两种顺序）；更新 `PermissionModal.test.tsx` 中 2 处 ask_user 断言（输入框改在 pane 内末项；单选选后打字 → selected []）；更新 `turnStore.test.ts` normalize 期望去掉 free_text。审批卡（AskCard/approval）断言未动。

## 证据（/exec-daemon/node v22.14.0）
- `node node_modules/vitest/vitest.mjs run src/components/AskUserCard.test.tsx src/components/PermissionModal.test.tsx src/stores/turnStore.test.ts` → 3 files / 71 passed
- `node node_modules/vitest/vitest.mjs run`（web 全量）→ 170 files / 1239 passed
- `node node_modules/typescript/bin/tsc -b` → 0 错误（本包 0，既有 0）
- lint：web 无 lint script，node_modules 无 eslint/biome/oxlint → 未执行（既有缺失，非本包）

## 遗留
- 后端 WP-080/081 联调；desktop 壳手点核对（中层）。

# WP-083 · 末项常驻文案 + 选中态（补 WP-082）· 实施记录

owner frontend · approver frontend_mid · 范围仅 AskUserCard.tsx / AskUserCard.test.tsx · 未 commit

## 改动
- `AskUserCard.tsx`：末项改为 `<label data-testid="ask-user-other-${qid}">`，内含可见 span「其他（自己填写）」（输入框 `aria-labelledby` 指向它，名称稳定；placeholder 仅补充）+ 输入框。
- 选中态复用选项同一套 class 切换：`permission-card__option` + 选中 `btn-primary` / 未选 `btn-ghost` + `btn-sm`。选中 = trim 后有文本且（单选）无已选项；多选有文本即选中。
- 语义等价（中层已接受）：label `data-selected=true|false`；选中时输入框 `aria-describedby` → sr-only「已选中」。
- `AskUserCard.test.tsx`：新增 5 用例 a–e（文案常驻；单选打字选中；单选点选项取消末项；纯空白不选中；多选共存），断言可见文字/role/aria + btn-primary/btn-ghost 切换（单选、多选都覆盖）；WP-082 用例 1 改为断言 label 为组内末项。

## 证据（/exec-daemon/node v22.14.0）
- vitest AskUserCard.test.tsx + PermissionModal.test.tsx（含审批卡）+ turnStore.test.ts → 3 files / 76 passed
- vitest web 全量 → 170 files / 1244 passed
- `tsc -b` → 0 错误

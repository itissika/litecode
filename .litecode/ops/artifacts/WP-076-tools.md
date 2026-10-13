# WP-080 产物（Tools）：ask_user 去 allow_free_text（GM 方案 B）
父包 WP-077；approver=backend_mid；未 commit。改动仅 src/tools/ask_user.rs、src/tools/descriptions/ask_user.md。末项文案定为「其他（自己填写）」（逐字）。

## 落地
- R1 schema：删 questions[].allow_free_text 与顶层 legacy allow_free_text。
- R2 parse_questions：batch 与 legacy q0 均不再读 allow_free_text / free_text；旧 agent 传入静默忽略（validate 通过）；AskQuestion.free_text 恒 true。
- R3 描述：去 allow_free_text；写明系统为每题自动追加末项「其他（自己填写）」，劝阻 agent 自加 "Other"/"其他"（不拒绝）；说明无选项仅 free_text = 用户经「其他（自己填写）」作答。
- R4 format_answers：selected 空 + free_text 非空 → `[q] selected: (none) — answered via 「其他（自己填写）」` + `[q] free_text: …`，不输出 selected_ids、不判为无答案；selected 空且无文字仍为 `selected: (none)`；其余不变。不合成 __free_text__，末项不进 selected。
- R5 新测：schema_has_no_allow_free_text、legacy_free_text_flags_ignored_free_text_always_true、text_only_answer_is_formatted；原 6 测保留且绿。

## 验证
- cargo test --lib tools::ask_user → 9 passed
- cargo test --lib permission::grants → 4 passed（含 Session 的 wp081_legacy_q0_free_text_always_true）
- cargo test --lib --no-run → Finished，无 error
- rustfmt --check src/tools/ask_user.rs 干净

## 与 WP-081（Session）交叉核
- AskQuestion.free_text：两边一致——字段保留、ask_user 恒 true；Session 已在 grants.rs effective_questions q0 路径改恒 true（工作区已见），本包在解析侧恒 true，两条构造路径都覆盖。
- 答案语义：一致——AskAnswer{selected, free_text?} 不变，末项不进 selected；仅文字（selected 空 + free_text 非空）= 答完。CLI 方案单选末项 → selected=[] + free_text，与本包 format_answers 输出匹配。
- 小差异（非冲突）：WP-081 规定"选了末项但文字为空 → 未答"，这类答案到 Tools 侧表现为 selected 空且无 free_text，渲染为 `selected: (none)`，与"未答"一致。
- 依赖提醒：仅文字作答要端到端可用还需前端放宽 allAnswered（WP-081 §4 风险②，前端域）。

# WP-022 / WP-026 — 上下文域：R3 sources 单测 · 可疑实现审计 · 未提交文件 · 测试裁剪 · Runtime 互审

- owner: context　approver: backend_mid　状态：执行完成，待 deliver
- 源码状态：HEAD `c6b24df` + 未提交；本域 diff stat `6 files changed, 17 insertions(+), 10 deletions(-)` + 未跟踪 `src/reminder/sources/tests.rs`
- 工具链：`PATH=/home/box/.cargo/bin:$PATH`，`rustc 1.99.0 (b940084d7 2026-09-28)`；所有证据均为真实 cargo 运行（共享 target 目录，排队等锁，未杀进程）。
- 不 commit；未触碰引擎产品 / symlink 复制；产品代码零改动。

## (a) R3 — reminder/sources 最小专属单测
文件 `src/reminder/sources/tests.rs`（经 `src/reminder/sources/mod.rs:21-22` 的 `#[cfg(test)] mod tests;` 挂载）。9 个 source 各有正向与抑制用例：

| source | 测试 | 覆盖行为 |
|---|---|---|
| tasks | tasks_restore_only_after_compaction_and_once | 无 compacted head 不发；head 后只发一次；空不发 |
| background | background_restore_gates_on_compaction_and_content | 同上 + 文案计数 |
| files_changed | files_changed_caps_at_twenty_with_more_count | 空不发；23→20+"and 3 more"；**新增**：正好 20 条不出 more 行 |
| bash_exit | bash_exit_distinguishes_kill_and_lists_running | 无 exit 不发；Kill 与正常退出文案；running: 0 |
| subagent_settled | subagent_settled_detail_overrides_listing | 空不发；detail 覆盖列表；**新增**：agent 非空时输出 agent 行 |
| custom_tool_settled | custom_tool_settled_lists_entries_and_skips_empty_detail | 空不发；空 detail 不输出 |
| plan_changed | plan_changed_diff_against_seen_and_spine | seen==disk 不发；spine 已宣告同 revision 不发；空 revision 不发 |
| step_budget | step_budget_warns_within_three_once_per_turn | 无上限/剩 4 不发；剩 3 发；同回合不重复；新回合再发 |
| env | env_emits_once_until_snapshot_changes | 首次发；快照不变不发；cwd 变化重发 |

本 WP 的改动：头注释 WP-018→WP-026；补 2 条断言（见上）；rustfmt。

证据（真实运行）：
- `cargo test --lib reminder::sources::` → **9 passed; 0 failed**
- `cargo test --lib reminder::` → **34 passed; 0 failed**
- `cargo test --lib context_pipeline` → **47 passed; 0 failed**
- `cargo test --lib death_list`（src/types/death_list_gate.rs）→ **13 passed; 0 failed**
- `cargo test --test stage_a_ctx_consistency` → **27 passed; 0 failed**

## (b) 可疑实现审计（只读；分级用 grade × layer；B 列为阻断判断）
| # | 区域 | 发现 | file:line | 分级 | 阻断 | 处置 |
|---|---|---|---|---|---|---|
| 1 | compact 失败恢复 | Compact 已落库后 reload 失败：rows 回滚到 snapshot 且 emit Failed，与库不一致 | context_pipeline/compact.rs:283-323 | bug/product | 否（低概率，下一次 sync 靠 cursor 自愈） | **ISSUE-035** |
| 1b | compact 并发 | `expected_revision` 在提交时现取（`revision_blocking().unwrap_or(0)`），不是快照时取；乐观锁失效，只剩 expected_prefix 兜底；读失败退化为 0 | compact.rs:286 | suspect/product | 否 | 并入 ISSUE-035 证据 |
| 1c | compact 取消 | auto compact 成功后若 cancel，`compact_if_needed`/`compact_step` 返回 Ok(false) 且不安装新 rows；"single source of truth" 注释被违反，pipeline state.working 暂留旧行 | compact.rs:150-152; context_pipeline/mod.rs:290-292 | suspect/product | 否（下一回合 begin_turn 重载） | 提交 Runtime 互审 |
| 1d | compact 取消→结局 | 压缩中取消返回 Err(Canceled)，agent/core.rs 统一映射为 TurnOutcome::Error | compact.rs:252-256, 415-432; agent/core.rs:43-46 | bug/product | 否 | **ISSUE-033**（owner runtime） |
| 1e | stale plan | `let _ = sessions.settle_stale_plan` 吞错（手动路径） | compact.rs:63 | suspect/product | 否 | 记录；auto 路径 exec.rs:98 有 warn |
| 2 | prepared view 生命周期 | compact_step 不清 `state.prepared`；在 compact_step 与 build_view 之间，current_token_estimate 返回压缩前的旧计数 | mod.rs:294-301, 440-452 | suspect/product | 否 | 记录 |
| 2b | 锁粒度 | commit_step_with_turn 分两次加锁读 log_max_seq 与 turn_id，可被撕裂 | mod.rs:394-406 | suspect/product | 否（单回合内串行调用） | 记录 |
| 2c | 静默重载失败 | sync_turn_working 在 working_set_blocking 出错时直接 return，继续用旧 working | mod.rs:228-230 | suspect/product | 否 | 记录 |
| 3 | seam 写入顺序 | 改动文件在 seam 开头即消费；spine view 失败或 FilesChanged 追加失败都不回灌，通知丢失 | runtime/exec.rs:294, 386-388, 462-467; reminder/file_tracker.rs:66-81 | bug/product | 否 | **ISSUE-032**（owner runtime；file_tracker 需配合提供 restore） |
| 3b | seam 顺序 | engine::sync 固定顺序只追加，与 DATA.md 一致 | reminder/engine.rs:107-137 | — | — | A（一致） |
| 4 | file_tracker | 进程内存、per-session 锁（manager.rs:1389,1399）保护，没有并发问题；锁中毒时静默返回空 | reminder/file_tracker.rs:31-81; session/manager.rs:1388-1406 | suspect/env | 否 | 记录 |
| 5 | migrate | 纯函数，无 IO；回滚由调用方 schema.rs:277/304 的事务负责（不属本域） | reminder/migrate.rs:15,46 | — | — | A；schema 事务请 Session 域确认 |
| 6 | mentions 去重 | 去重键用原始 path 字符串，`src/a.rs` 与 `./src/a.rs` 会重复附加 | reminder/mentions.rs:72-88 | suspect/product | 否 | 记录 |
| 7 | budget 硬上限 | 越限即返回 TokenBudgetExceeded（不截断）；baseline 只按 item 数比较，回退后再追加到同长度时会误用旧前缀 | context_pipeline/budget.rs:112-125, 143-153 | suspect/product | 否 | 记录 |

bug 级新开 issue：ISSUE-035、ISSUE-032、ISSUE-033（ops validate OK）。suspect 级只记录在本表，不开 issue。

## (c) 本域未提交文件
| 文件 | 内容 | 来源 | 处置 |
|---|---|---|---|
| context_pipeline/compact.rs | `fetch_add` 合并为一行（测试代码） | rustfmt | 保留 |
| reminder/engine.rs | use 换行；测试缩进修正 | rustfmt | 保留 |
| reminder/kinds.rs | 删一个空行 | rustfmt | 保留 |
| reminder/mentions.rs | 测试中 fs::write 换行 | rustfmt | 保留 |
| reminder/mod.rs | re-export 重排 | rustfmt | 保留 |
| reminder/sources/mod.rs | `#[cfg(test)] mod tests;` | WP-018→R3 | 保留 |
| reminder/sources/tests.rs（未跟踪） | R3 单测 | WP-018 起草，WP-026 收口 | 保留 |
全部随 GM 批量提交；不 revert（approver 指示保留 fmt 改动）。

## (d) 测试裁剪清单（只列不删）
| 测试 | 建议 | 理由 / 替代覆盖 |
|---|---|---|
| reminder/engine.rs `bash_exit_names_a_user_kill` | 重复 | sources::tests::bash_exit_distinguishes_kill_and_lists_running 覆盖更全；engine 层可只保留顺序/组合测试 |
| reminder/engine.rs `custom_tool_settled_names_outcomes` | 重复 | sources::tests::custom_tool_settled_* |
| reminder/engine.rs `plan_changed_dedupes_on_revision` | 重复 | sources::tests::plan_changed_diff_against_seen_and_spine |
| reminder/engine.rs `step_budget_once_per_turn_inside_the_window` | 重复 | sources::tests::step_budget_warns_within_three_once_per_turn |
| reminder/engine.rs `restore_fires_once_after_compaction` | 保留但可收窄 | 它还覆盖 view_from_spine→sync 链路，仍有价值；可去掉与 sources 重复的文案断言 |
| budget.rs `token_count_prefers_provider_when_set` / `..._falls_back_to_estimate_without_provider` | 可合并 | 一组二分支，可并为一个表驱动测试 |
| keep_recent.rs `entire_transcript_within_keep_returns_none` / `empty_returns_none` | 可合并 | 同一 None 分支 |
| compact.rs 测试里 `compact_summary_drops_reasoning_items` / `compact_summary_empty_when_only_reasoning` | 保留 | 边界不同 |
| death_list_gate.rs（13） | **不动** | 死名单；任何改动须先开 issue |
| stage_a / working_set_identity / turn_prefix_stability | 保留 | 跨域行为门，无重复 |
本 WP 不删除、不放宽任何测试。

## (e) 与 Runtime 互审：compact/reminder ↔ 回合交界
### 我对 Runtime 侧的检查清单与读取结果
1. **compact 触发**：agent/core.rs:39 seam → :43 `compact_if_needed` → :47 压缩后再跑一次 seam → :50 prepare_view。exec.rs:89-119 → `ContextPipeline::compact_step`（mod.rs:255）→ compact.rs:93。触发条件为 token > 80%（budget.rs:131）。✔ 符合"恢复类提醒落在 summary 之后"的口径（mod.rs:250-254）。⚠ exec.rs:30 注释"compact_if_needed already ran prepare_step"已过时（drift/product）。
2. **失败→TurnOutcome 映射**：compact 返回的任何 Err 都映射为 TurnOutcome::Error（core.rs:45），符合"auto 失败→Error"。✘ 但 Canceled 也走这条路 → ISSUE-033。manual 路径（compact_now，compact.rs:38）不经 core.rs，由调用方处理，可重试 ✔。
3. **cancel 传播**：exec.rs:90 先检查；compact.rs:109/118/150/181/252/350/417-431 多处检查；LLM 等待使用 select + 超时（compact.rs:407-455）。✘ 结局映射见 ISSUE-033；⚠ 压缩成功后再取消会返回 false（见 b-1c）。
4. **seam facts 收集**：exec.rs:255-484。bash/subagent/custom mailbox 在失败时回灌 ✔（:386-388, :450-481）；✘ changed files 不回灌 → ISSUE-032；非 subagent 类提醒追加失败只记 warn，不报错（:462-467），请 Runtime 确认这是有意的口径。

### 请 Runtime 审我方
1. compact 返回契约：`Result<bool>` 的语义（compact.rs:105-108, 166-167），错误种类有 NothingToCompact / Canceled / TokenBudgetExceeded / CompactionFailed / Llm / ToolExecution，以及 compact_fail_kind 映射（compact.rs:475-481）。
2. 失败时的内存回滚：每个 Err 分支都执行 `*rows = snapshot`（compact.rs:196, 203, 246, 253, 266, 294, 311），以及 ISSUE-035 的 reload 分支。
3. compact_step 成功后安装 rows 和 log_max_seq（mod.rs:294-301）；取消后不安装（mod.rs:290-292），是否接受？
4. prepared view 失效条件：begin_turn/end_turn 清空（mod.rs:197, 247），Idempotent commit 清空（mod.rs:428-430），compact 不清空（b-2）。
5. reminder 注入位置和顺序：engine::sync 的固定顺序（engine.rs:107-137）；seam 在 compact 前后各跑一次时，restore 只发一次（tasks.rs:5-10、background.rs:5-13 依赖 kinds_after_compacted）。
6. file_tracker 的 absorb/take_changes 语义（file_tracker.rs:56-81），以及 ISSUE-032 修复时需要的 restore 接口由哪一方提供。

有分歧时在对应 ISSUE 中追加 history。

## 工具需求
无。

## 交叉审（Runtime）
审查人：Runtime；基线：ops/team-framework 工作区（未提交），只读核对，无代码改动。

| # | 审查点 | 证据 | 结论 |
|---|---|---|---|
| 1 | compact 返回契约 | compact.rs:105-110 取消→Ok(false)；166-167/180-197 无可裁→Ok(false)；337 成功→Ok(true)；475-481 仅 NothingToCompact/Canceled 单列，其余→Failed | 同意。补充：`compact_transcript` 入口取消返回 Ok(false)，而中途取消（254）返回 Err(Canceled)，同一取消两种形态，正是 ISSUE-033 的来源 |
| 2 | Err 分支回滚 | 196/203/246/253/266/294/311 均先 `*rows = snapshot` 再发事件/返回；snapshot 在 185 于 split_off 之前取，tail 一并恢复 | 同意，7 处齐全。ISSUE-035 成立；补充：Compact mutation 带 `expected_prefix`（291）仍能挡住前缀被改的竞态，revision 现取（286）的危害较小，严重度维持 bug、建议非阻断（已追加 history） |
| 3 | compact_step 取消后不安装 | mod.rs:290-292 取消即 Ok(false)；294-301 才安装 | 接受（minor）。若 compact 已落库并发 Succeeded 后才取消，返回 Ok(false) 与事件不一致、内存与 log_max_seq 暂旧；但 core.rs 下一检查点走 Cancelled，且 build_view/begin_turn 经 sync_turn_working（mod.rs:316）按 cursor 重载自愈，无持久不一致。建议注释写明 |
| 4 | prepared view 失效 | mod.rs:197/247 清；428-430 按 clear_prepared 清；compact 不清 | 同意语义正确：compact 后 core.rs:47-50 必然 re-sync + prepare_view，build_view 在 342 覆盖 prepared。仅 compact→prepare 之间 `current_token_estimate`（mod.rs:440-450）会读到旧 token 数，minor，不立 issue |
| 5 | engine::sync 顺序/restore 只发一次 | engine.rs:107-136 固定顺序 env→tasks→background→plan_changed→files_changed→bash_exit→subagent→custom→step_budget；tasks.rs:5-10、background.rs:5-13 以 compacted_head + kinds_after_compacted 去重 | 同意。前提是第一次 seam 的 append 已成功落库；append 失败只 warn（exec.rs:462-467）时，下一 seam 会重发 tasks/background——这对可再生类型恰是期望行为 |
| 6 | file_tracker 语义 | file_tracker.rs:57-62 absorb 全量重快照；66-81 take_changes 读取即覆盖快照、无回滚 | 同意语义描述。ISSUE-032 归属：建议拆分——上下文（file_tracker 属主）提供 `forget/mark_dirty(paths)`（删 snap 使 `is_none_or` 下次报 differs），Session 在 SessionManager 暴露 `restore_changed_files(session_id, paths)`（与 manager.rs:1418 对称），Runtime 在 exec.rs 386 与 FilesChanged append 失败分支调用；issue owner 仍为 runtime（已追加 history） |

对 Runtime 代码读取的确认：
- ISSUE-033：确认。core.rs:43-46 `Err(e) => TurnOutcome::Error(e)` 未区分 Canceled，对照 core.rs:56-58 call_model 正确映射 Cancelled；compact.rs:252-256 取消还会发 Failed 生命周期（compact_fail_kind→Canceled）。同意 bug，修复在 Runtime：core.rs:45 增加 `Err(LitecodeError::Canceled) => Cancelled`。注意与 WP-025 的 auto compact→compaction_failed 映射（project.rs:537-563）互不冲突，Canceled 不应进入该映射。
- ISSUE-032：确认。exec.rs:294 先消费；386-388 restore_seam_sources（486-499）只回灌 bash/subagent/custom；462-467 非 subagent 失败只 warn。
- exec.rs:30 注释过时：确认。prepared view 由 core.rs:50 `prepare_view` 生成，不是 compact_if_needed；应改为“prepare_view 已生成 PreparedView”，31-34 的错误文案同理（trivial）。
- 非 subagent 提醒追加失败只 warn：部分有意。env/tasks/background/plan/step_budget 可由下一 seam 重新推导，warn 可接受；bash/custom 在 471-481 已回灌，不丢；唯一不可再生的是 files_changed，即 ISSUE-032。建议在 462 旁补注释说明“可再生类型 warn、不可再生类型须回灌”。

结论：6 条全部同意，无阻断；分歧仅为 ISSUE-032 接口归属细化与 ISSUE-035 严重度说明，已在对应 ISSUE 追加 history。

# WP-207 G1 — ptc 栈溢出封口（Runtime，草稿；未编译、未测）

## 根因
- `engine.rs:273` `set_max_stack_size(stack_bytes)` 与 `arm_stack_watch`（limit = base − stack_bytes）用同一字节数；且 `arm_stack_watch` 先 `JS_UpdateStackTop`，两者基准点相同 → 两个阈值重合。
- QuickJS 原生检查在每次调用都做（`js_check_stack_overflow`），抛 `JS_ThrowStackOverflow` → 普通 `RangeError("Maximum call stack size exceeded")`（quickjs.c:8567），可 try/catch；自研 watch 只在 `js_poll_interrupts` 点（含 JS_CallInternal 入口 quickjs.c:18074）采样，本质晚一步 → 原生先触发，`Faults::STACK` 未置位，分类为 Exception 且可被吞。

## 修法
A（主）：原生上限 = `native_stack_bytes(stack_bytes)` = `stack_bytes + max(64KiB, stack_bytes/2)`，封顶 `1<<24`（rquickjs 上限）；stack_bytes=0/超上限 → 原样（watch 也不 arm）。距 watch 阈值 48KiB 内 watch 每次调用都 poll，故 watch 必在原生之前 latch 并 `return true`（不可捕获）→ classify 走既有 `range proto && stack_fault` → `ResourceLimit/Stack`，或 interrupt 路径 `Faults::STACK` → "stack overflow"。
B（后备）：`note_stack` 一旦 latch，后续每个 poll 点都把计数器设 1；interrupt handler 见 latch 即 `Faults::STACK` 返回 true。即使原生 RangeError 先抛且被 JS catch，catch 后第一处 poll（调用/循环回边）即封口并归类 Stack。**不**按消息文本识别（tests.rs:1176 用户 throw 同名消息须仍为 Exception）。C（终检，mid 补刀）：`drive` 在 `faults.disarm()` 之前调用 `seal_latched_stack(end, &faults)`：只要 latch（仅由 `note_stack` 在真实栈指针越过 watch 阈值时置位，不看消息）为真，任何结局（Success / CancelledAfterReturn / 非 Stack 的 Failure）一律改写为 `ResourceLimit/Stack "stack overflow"`；已是 Stack 的保持原样。用户自 throw 同名消息不会置 latch → tests.rs:1176 用例不受影响。
**残余风险（catch 后不经 poll 点直接 return）已关闭。**
线程栈：`ptc-js` 线程 `stack_size = native + 2MiB`（`js_thread_stack_bytes`）；原本用 Rust 默认 2MiB，若 stack_bytes 接近 1<<24 会先撞 OS guard page。

## 数值
| stack_bytes | watch | 原生上限 | 线程栈 |
|---|---|---|---|
| 64KiB（两测） | 64KiB | 128KiB | 2MiB+128KiB |
| 256KiB（tests limits()） | 256KiB | 384KiB | 2MiB+384KiB |
| 512KiB（Limits::default） | 512KiB | 768KiB | 2MiB+768KiB |
| 16MiB（上限） | 16MiB | 16MiB（封顶，headroom 0 → 只有 B 兜底） | 18MiB |
| 0 | 不 arm | 0（无） | 平台默认 |
注：≥ (16MiB − 64KiB) 时 headroom 被封顶压缩，此区间依赖 B。

## 改动文件
src/ptc/engine.rs（set_max_stack_size、note_stack latch、seal_latched_stack 终检、native_stack_bytes / js_thread_stack_bytes / 常量）、src/ptc/mod.rs（线程 stack_size）、src/ptc/tests.rs（仅新增 1 条：`stack_overflow_caught_then_immediate_return_is_still_sealed`，其余未动）。

## 待跑 cargo（放行后，free≥2GiB，CARGO_BUILD_JOBS=2）
1. `cargo test --lib -j 2 ptc::tests::stack 2>&1` —— 覆盖 stack_limit、stack_overflow_stays_sealed_when_the_program_catches_it、新增 stack_overflow_caught_then_immediate_return_is_still_sealed：`cargo test --lib -j 2 ptc::tests::stack 2>&1 | tee .litecode/ops/artifacts/WP-207/cargo-G1-stack.txt`
2. `cargo test --lib -j 2 ptc:: 2>&1 | tee .litecode/ops/artifacts/WP-207/cargo-G1-ptc.txt`（含 tests.rs:1176 伪造消息用例、memory、deep nesting）
3. `cargo test --lib -j 2 tools::code 2>&1 | tee .litecode/ops/artifacts/WP-207/cargo-G1-code.txt`
4. `cargo clippy --lib -j 2 2>&1 | tee .litecode/ops/artifacts/WP-207/cargo-G1-clippy.txt`

## G1 结果
cargo-G1-stack.txt 3/3 ✅（stack_limit、stack_overflow_stays_sealed…、新增 stack_overflow_caught_then_immediate_return_is_still_sealed）；cargo-G1-ptc.txt 58/58 ✅（含 1176 用户同名 throw 用例）；cargo-G1-code.txt tools::code 48/48 ✅；cargo-G1-clippy.txt exit 0（ptc 处仅既有 result_large_err 警告）。commit 99ac65f。

## 附录：phase2b/phase3 六条红 × WP-206 A（只读分析）
A 的作用面：仅 `releases_lock_after_spawn()==true` 的工具（只有 `run`）改变锁生命周期；`RunScope` 对其他工具是透明包裹（guards 仍持有到调用结束，与改前同）；`deadline` 只被 `run` 读取；30s 窗口只在 run 的 PipeDetach。下列 6 条均不使用 `run`（GateTool/LateTool/ReadTool/WriteTool），故**与 A 无关**。证据来自 cargo-full-pre.txt（HEAD a8612cf）与测试源码；未另起 fb5e986^ worktree 对照（结论由代码路径即可确定，省 cargo）。

| 测名 | 失败断言 | 失败现象 | 与 A 关系 | 判定 | 建议 |
|---|---|---|---|---|---|
| phase2b cancellation_seals_in_flight_nested_calls | :431 `content.contains("1. block cancelled")`（之前 `sealed` 断言已过） | 输出 `run_code failed: cancelled · 0 succeeded…`，无 ledger 行 | 无 | 过时（WP-200/201 去 ledger 行；封口语义本身 OK） | 改断言为 head/data（cancelled、unsettled=0）或并入 ptc/code 单测后删 |
| phase2b outer_timeout_does_not_overwrite_a_completed_nested_result | :543 `contains("1. fast ok")`（`sealed` 已过） | `timeout after 1000ms · 1 succeeded, 0 failed` —— fast 计入 succeeded，正是被测语义 | 无 | 过时（ledger 文本） | 改为断言 `1 succeeded` + Timeout kind，或删 |
| phase2b a_call_that_outlasts_the_grace_period_is_unsettled | :474 `contains("unsettled")` | 有 "1 calls were still running … may still land"，但新 head 不再出现单词 "unsettled"；3402ms = 400ms 预算 + ~3s grace 等待 LateTool（sleep 4s），属设计行为 | 无 | 过时（文案） | 断言改 "still running"/data.unsettled，或删（ptc 有 unsettled 单测） |
| phase2b approval_wait_does_not_consume_the_program_budget | :510 结果 `"ok"`，得 `"missing"` | `result.error` 为 None（预算未被审批吃掉——被测语义成立）；脚本用 `read.file` 判断，而 read data 已无 `file` 字段（read.rs:725/899 明确 retired） | 无 | 过时（read data 形状） | 脚本改 `read.lines`/`read.path`；保留（审批暂停预算是有效语义） |
| phase3 conflicting_spellings…_a_read_waits_for_the_write | :1032 `held.saw == true` | `saw:false`；reads 报 file not found（这本身是测试**期望**的：审批期间 read 不应等写）。`saw` 由 `markerText()`→`lineText()` 读 `result.lines[0].text`，而 read 的 `lines` 已是字符串数组（read.rs:393 `lines.push(text)`）→ 恒为 "" | 无 | 过时（read data 形状），非产品缺陷 | 改 `lineText` 为 `result.lines[0]`；保留（create_only 竞态/审批不加锁是有效语义） |
| phase3 two_nested_writes_leave_one_complete_file… | :496 `contains("write path=same.txt ok")` | 文件断言全过（same 为完整体、seq=two、other=CCCC），输出 `5 succeeded, 0 failed`，无 ledger 行 | 无 | 过时（ledger 文本） | 删 ledger 断言或改 `5 succeeded`；保留并发写一致性部分 |

结论：6 条均为测试过时（ledger 文本 / read data 形状），被测产品语义（取消封口、超时不覆盖已完成结果、grace 后 unsettled、审批不耗预算、审批期间不预占锁、并发写完整）从现象看均成立；与 WP-206 A 无关；无产品缺陷。

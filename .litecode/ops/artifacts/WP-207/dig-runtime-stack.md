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

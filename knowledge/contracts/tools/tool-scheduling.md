```node
node : tool-scheduling
status : pending
summary : 并发安全调用仅在末批同样安全且资源不冲突时合并，否则开新批；非并发安全独占一批，跨会话锁只覆盖 write/edit。
x : 2556
y : 504
w : 398
h : 536
```

**主张：** executor 按 `is_concurrency_safe` 与 `resource_keys` 分批：并发安全调用仅在末批同样安全且资源不冲突时合并，否则开新批；非并发安全调用独占一批，批间顺序执行（冲突与末批全体调用比对，不限相邻一对）。

- 声明默认：`is_concurrency_safe` 默认 false（串行）、`resource_keys` 默认空 [@ file="src/tool/trait_.rs" symbol="trait Tool › fn is_concurrency_safe"]、[@ file="src/tool/trait_.rs" symbol="trait Tool › fn resource_keys"]；read 恒并发安全 [@ file="src/tools/read.rs" symbol="impl Tool for ReadTool › fn is_concurrency_safe"]，bash 仅只读命令为 true [@ file="src/tools/bash.rs" symbol="impl Tool for BashTool › fn is_concurrency_safe"]。
- `partition_tool_calls` 只把并发安全且与上一批无资源冲突的调用并入上一批，否则新开一批 [@ file="src/tool/executor.rs" symbol="fn partition_tool_calls"]；批内 `tokio::spawn` 并发，批间顺序 await（非并发安全批内逐个 await）[@ file="src/tool/pipeline.rs" symbol="impl ToolPipeline › fn execute_batch_cancellable"]。
- 跨会话加锁时才做键过滤：`cross_session_lock_keys` 按工具名保留 write/edit 的整套键，bash/read 的键不参与跨会话锁、只用于同轮分批冲突判定 [@ file="src/tool/executor.rs" symbol="fn cross_session_lock_keys"]。
- 锁是进程级单例（`process_write_lock`）：`try_acquire` 全有或全无、同会话可重入、冲突即返回持有者 session_id（调用 fail fast 报 resource busy，不排队）；`WriteLockGuard` 返回时 `release_all` [@ file="src/tool/write_lock.rs" symbol="impl WorkspaceWriteLock › fn try_acquire"]、[@ file="src/tool/write_lock.rs" symbol="fn process_write_lock"]、[@ file="src/tool/executor.rs" symbol="impl Drop for WriteLockGuard"]。
- `ResourceKey::Workspace` 粗粒度键仍存在，但生产 bash 不再获取它 [@ file="src/tool/write_lock.rs" symbol="enum ResourceKey"]。

相关：[@ key="tool-control-boundary"]、[@ key="tool-execution-context"]。

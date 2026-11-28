```node
node : tool-scheduling
status : enabled
summary : 并发不靠猜：工具声明读谁写谁，调度照声明分批、照声明加锁；写独占、读共享，跨会话抢不到就当场报冲突。
x : 2124
y : 594
w : 398
h : 536
domain : tools-permission
```

**一句话：** 一次调用会不会碰到别人，工具自己说了算——它声明读哪些、写哪些，调度就按这份声明分批和加锁。

**关键点：分批看声明，加锁也看同一份声明**

- 有写的调用不并发：声明里带写，就单独占一批；没有声明的老工具才回落到 `is_concurrency_safe`（默认 false，等于串行）。合并也不是塞进上一批就完，要和那一批里的每一个都不一样才算安全（[partition_tool_calls](src/tool/executor.rs#partition_tool_calls)、[Tool.effects](src/tool/trait_.rs#Tool.effects)）。
- 执行前过两道门：先排进程内的读写队列（读共享、写独占、每个键先来先服务、多个键按排序获取以免互相卡死；没声明资源就直接过），再抢跨会话写锁，抢不到当场报 `resource busy` 并说出是谁占着，不排队干等（[ResourceGate.acquire](src/tool/resource_gate.rs#ResourceGate.acquire)、[WorkspaceWriteLock.try_acquire](src/tool/write_lock.rs#WorkspaceWriteLock.try_acquire)）。
- 锁不跟着长命令一起耗：`exec_command` 在子进程 spawn 成功后就把锁交出去，长跑的命令不会一直占着工作区。

**不变量：** 同一个文件同一时刻只能有一个写者（跨会话也一样），写的时候别人既不能写也不能读；读与读可以并行。

上游：[tool-control-boundary](knowledge/contracts/tools/tool-control-boundary.md)。
下游：[tool-execution-context](knowledge/contracts/tools/tool-execution-context.md)、[tool-call-lifecycle](knowledge/contracts/tools/tool-call-lifecycle.md)。

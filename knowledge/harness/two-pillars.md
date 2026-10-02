```node
node : two-pillars
status : pending
summary : L1 二支柱：Context 管「看到什么」、Tool 管「能做什么」；单点入口、管线不得旁路。
x : 1224
y : 1044
```

**L1 只有两根支柱，agent 能力从这两处进入。**

- **Context** —— 组装每次模型调用看到的内容；视图是计算产物，不是存储 [@ id="context-is-view" label="上下文即视图"]。
- **Tool** —— 唯一能力出口；内置 / Custom / MCP 同合约；只读并行、写/执行串行（工作区写锁）[@ id="tool-aci" label="Tool 统一抽象"] [@ file="src/tool/write_lock.rs" label="write_lock.rs"]。
- **管线** —— Tool 执行链上嵌入授权，禁止旁路：不能绕过出口整形、门闸与权限 [@ id="permission-asymmetric" label="权限模型"]。
- **分界** —— Session 持久化日志；Context 只在调用前准备视图 [@ id="session-log-truth" label="日志为真源"]。



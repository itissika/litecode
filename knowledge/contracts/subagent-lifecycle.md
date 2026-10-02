```node
node : subagent-lifecycle
status : pending
summary : 子会话契约：持久 parent/call 关系，独立 turn；Hub 只路由完成，父取消不隐式取消 child。
x : 990
y : 666
w : 398
h : 720
```

**subagent 是完整 session，调度工具是相同 turn 基座的另一种入口。**

### 数据
- child meta 保存 parent_session_id、parent_call_id、responsibility 与 depth；parent_call_id 把 child 挂到父侧 launch 调用。
- `CompletionRef` 只携带 parent、child、turn 身份；结果来自 child 的持久 turn 记录，不在 Hub 另存一份运行真相。

### 流转
launch 校验角色／父 allowlist／call 身份 → 建 child session → 绑定通知 → reserve／spawn／start → child 自己的事件通道 → TurnFinished → Hub inbox → 父 seam 写完成提醒。

### 不变量
- child 按自身绑定解析模型与上下文等执行配置，不由父 launch 临时指定或继承父模型。
- primary 与 child 复用会话写门和 turn 原语；底层等价不代表权限、生命周期所有权或 UI 入口相同。
- depth 从 child session meta 读取，上限一层；child 不暴露嵌套调度工具。模型不能靠参数绕过深度约束。
- Hub 只订阅 lifecycle 并路由完成引用，不持有另一套运行表、不代替 SessionManager finish／join。
- 父 turn 取消不传播为 child 取消；停止 child 必须显式走对应 session 的 cancel，child 后续仍可继续执行。
- 完成不是隐形 prompt 注入：运行中的父在 seam 落完成提醒；空闲父满足自动唤醒条件时启动新 turn 再消费。

依据：[@ file="src/tools/subagent/launch.rs" label="创建与父子边界"]、[@ file="src/tools/subagent/turn.rs" symbol="fn start_turn_like_human" label="共用 turn 原语"]、[@ file="src/tools/subagent/hub.rs" symbol="fn spawn_completion_router" label="只路由完成"]、[@ file="src/runtime/subagent_auto_turn.rs" label="空闲父唤醒"]。
验证：[@ file="src/tools/subagent/spawn_contract.rs" label="child 配置与提供方测试"]、[@ file="tests/subagent_session_isolation.rs" label="会话隔离测试"]。

上层：[@ id="featured-tools" label="特色工具"]。

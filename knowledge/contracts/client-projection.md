```node
node : client-projection
status : pending
summary : 客户端投影：按 session/seq 镜像权威行，快照与增量补缺口；乐观气泡没有日志身份。
x : 272
y : 674
w : 398
h : 720
```

**客户端可以缓存与乐观展示，不能把这些呈现变成另一份会话真源。**

### 数据
- `WireBufferEvent` 传递 kind、body、state、seq；UI 的 HumanRow 再派生为 RenderNode。
- snapshot 分开持久 meta、进程 turn 与 buffer 游标；不同 session 的镜像和运行状态分片保存。
- last_seq 是存活日志尾部，next_seq 是不回退的分配高水位，revision 是会话版本；change_id 用于索引追平，settings_revision 是配置世代，不能互换。

### 流转
内核提交／运行事件 → 协议投影 → 订阅增量及独立 snapshot RPC → 冷启动 buffer/load／补缺口 → 按 seq upsert → 界面渲染。

### 不变量
- 同 session 的同 seq 增量整行替换，不靠文本合并；客户端依赖后端 settled 不可变契约，自己没有“final 拒绝覆写”守卫。
- 行的 live 状态由日志 state 决定，不从 provider payload.status 猜；隐藏行仍传输以保留游标语义。
- 乐观用户气泡／排队气泡不获得永久 seq 或回退锚点；只有落库行才成为事实。断线本身不保证清掉所有 pending 展示。
- 回退依据 last_seq 删掉本地旧尾部，不能用不回退的 next_seq 当存活边界；恢复时按 next_seq 补窗口缺口，重复加载按 seq 幂等。
- 断线清订阅状态，重连重新订阅并从 snapshot 恢复；订阅成功不等于已拿到历史，服务端先 flush 缓冲增量再回订阅确认。
- 子会话事件按自身 session 分发，不混入父 turn；关闭 tab／失去连接不拥有取消执行的权力。

依据：[@ file="src/client_protocol/protocol.rs" symbol="struct BufferState" label="双游标"]、[@ file="src/client_protocol/project.rs" symbol="fn buffer_log_row" label="行协议投影"]、[@ file="web/src/stores/messageStore.ts" label="seq 镜像与乐观态"]、[@ file="web/src/stores/sessionStore.ts" label="快照恢复与回退"]、[@ file="web/src/lib/transcriptProjection.ts" label="渲染投影"]。
验证：[@ file="web/src/stores/messageStore.test.ts" label="同 seq 替换测试"]、[@ file="web/src/stores/sessionStore.lifecycle.test.ts" label="恢复与回退测试"]。

上层：[@ id="ux" label="ux"]。

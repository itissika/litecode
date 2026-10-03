```node
node : client-projection
status : enabled
summary : 客户端投影：按 session/seq 镜像权威行，快照与增量补缺口；乐观气泡没有日志身份。
x : 270
y : 558
w : 416
h : 414
```

**主张：** 客户端可以缓存与乐观展示，不能把这些呈现变成另一份会话真源。

- 镜像按身份：按 (session, seq) 镜像权威行、同 seq 整行替换；行的生命周期取自日志 state，缺失即判非法，绝不猜「已定稿」。
- 双游标：存活尾部与分配高水位是两个游标——回退按存活尾部删尾、恢复按高水位补缺口，不可互换。
- 乐观有度：排队与乐观气泡不获得永久身份，只有落库行成为事实；断线不保证清掉 pending 展示。
- 恢复幂等：重连按快照加增量补缺口、重复加载按 seq 幂等；陈旧快照不得把已删行拉回。
- 纪律有测试：死亡清单测试封禁第二套对话语义；流式平滑只是显示缓冲。
- 边界：子会话事件按自身 session 分发；关闭 tab 或断开连接不拥有取消执行的权力。

上层：[@ key="ux"]。
依据：[@ file="src/client_protocol/protocol.rs"]、[@ file="web/src/stores/messageStore.ts"]。

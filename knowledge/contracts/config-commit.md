```node
node : config-commit
status : enabled
summary : 配置提交：持久意图与运行投影分面；generation 是提交世代，不是 CAS；保存与应用分阶段。
x : 510
y : 700
w : 416
h : 432
```

**主张：** 配置保存表达人的意图；保存成功不等于所有消费者已经应用。

- 唯一写门：UI / CLI / API 的所有设置写路径共用同一个写门，数据库与 `.litecode/*.json` 只是它的存储后端。
- 提交才有消费者：提交推进进程内 generation 并广播已提交文档；generation 是提交世代，不是乐观锁，不做冲突校验。
- 按文档应用：消费者按提交的文档决定重载范围——只有含引擎文档才触发引擎 reconcile；MCP 启动只按 id 读已存定义。
- 活动 turn 拒写：turn 运行期间拒绝设置写入，避免中途改变这次执行的配置意图。
- 分阶段语义：保存成功与应用成功是两件事；跨介质（文件 + 数据库）写入不是事务，不承诺「错误必无变更」。
- 目录是构建事实：provider / model 目录随构建内嵌，不是可写设置文档。

上层：[@ id="config" label="config"]。
依据：[@ file="src/config/settings_writer.rs" label="写门与混合写顺序"]、[@ file="src/config/gate/mod.rs" label="文档与应用范围"]。

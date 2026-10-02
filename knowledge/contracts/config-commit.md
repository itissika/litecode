```node
node : config-commit
status : pending
summary : 配置提交：持久意图与运行投影分面；generation 是提交世代，不是 CAS；保存与应用分阶段。
x : 510
y : 674
w : 398
h : 720
```

**配置保存表达人的意图，保存成功不等于所有消费者已经成功应用。**

### 数据
- `PersistDoc = DocId` 标识可写文档；`EvalView` 标识可用工具、引擎、MCP、安装进度等只读投影。
- `CommitAck` 带 generation、实际提交的 docs 与 restart_required；generation 是进程内提交世代，不是 session revision。
- provider/model 事实来自 catalog，凭据来自配置数据库，不能将展示 DTO 整体反写成配置。

### 流转
UI／CLI／API → SettingsWriter 互斥写门及 turn guard → 写 DB／工作区文件 → 推进 generation、广播 docs → resolve／apply → owner 发布运行投影。

### 不变量
- 所有设置写入口共用 SettingsWriter；活动 turn 期间拒绝设置写入，避免中途改变这次执行的配置意图。
- generation 在保存后推进；不接收 expected-generation 做乐观冲突校验，不可与 session 写门的 CAS 混同。
- apply 依据 docs 决定重载范围；只有包含 Engines 才触发引擎 reconcile，普通设置更新不隐含启停引擎。
- HTTP 的 commit 成功后 apply 失败仅记录警告，不回滚保存，也不把它当保存失败返回；下一 turn 的装配与正在运行的 turn 分开。
- catalog 事实由进程加载，变更需重启，不属于普通 settings commit 的热更新路径。
- **文件与 DB 不是一个跨介质事务**：混合提交先写文件再写 DB；后续失败没有文件补偿。不能承诺“错误必无变更”或“generation 未变就文件没变”。

依据：[@ file="src/config/gate/mod.rs" symbol="enum DocId" label="可写文档"]、[@ file="src/config/settings_writer.rs" symbol="impl SettingsWriter › fn commit_partial" label="提交世代"]、[@ file="src/config/settings_writer.rs" symbol="impl SettingsWriter › fn commit_mixed" label="混合写边界"]、[@ file="src/runtime/mod.rs" symbol="impl RuntimeHandle › fn apply" label="按 docs 应用"]、[@ file="src/serve/settings.rs" symbol="fn reload_runtime_after_settings_write" label="应用失败语义"]。
验证：[@ file="tests/f2_lock_scope.rs" label="写锁防丢更新测试"]、[@ file="tests/settings_api.rs" label="设置 API 测试"]。

上层：[@ id="config" label="config"]。

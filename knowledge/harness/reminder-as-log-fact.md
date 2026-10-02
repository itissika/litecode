```node
node : reminder-as-log-fact
status : pending
summary : 提醒是会话事实：每类 reminder 以独立 seq 落库、最新一条参与视图，不做每步 ephemeral 注入。
x : 272
y : 1044
```

**系统提醒不是拼进 prompt 的隐形噪声，而是写进日志的会话事实。**

- 每类 reminder 在 step 边界以独立 seq 落库 [@ file="src/reminder/mod.rs" label="reminder/mod.rs"]。
- 视图按最新一条 kind 去重（latest_by_kind），避免逐条重放成噪音 [@ file="src/reminder/engine.rs" label="engine.rs"]。
- 取代旧模型：曾「每步 ephemeral 注入」「仅在 compact 检查点附带」——均已废弃。
- 好处：提醒可回溯、可被 UI 投影、可被 compact 统一治理；与会话日志同构 [@ id="session-log-truth" label="日志为真源"]。



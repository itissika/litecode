```node
node : context-compact
status : enabled
summary : 压缩只改模型看到的历史，不改库里的事实：库里追加一行，旧行原样留着，读的时候才折叠成摘要 + 保留段。
x : 5364
y : 612
domain : harness-core
```

**目标：** 压缩只改模型看到的历史，不改库里的事实——库里追加一行，旧行原样留着，读的时候才折叠成「摘要 + 保留段」。

**必须做到：**

- 库里只追加，不改写任何旧行；折叠发生在读取时：检查点之前换成摘要，之后原样保留。
- 落地只有一条路：keep-recent + LLM 摘要，写一个 `compacted` 检查点。
- 落库之后从 fold 重载工作集；不许在内存里拼「摘要 + 保留段」直接当结果。
- 提交和交付分开：写库成功是提交事实，读回并校验出可用 surface 才算成功。
- 提交前失败不写任何东西；已经落库的不回滚。
- 手动压缩失败只报错；session 每次取 surface 失败必须报错停。
- 正常 turn 结束或取消都不取消压缩；只有回退和删除会取消并等收尾。

**绝对不做：**

- 机械裁剪历史——那条路已经整体拆掉。
- 在内存里拼压缩结果，绕开库直接喂下一轮。
- 读不回来就报成功。
- 因为一轮对话结束就掐掉会话的压缩任务。

依据：[compact_input](src/context_pipeline/compact.rs#compact_input)、[commit_plan](src/context_pipeline/compact.rs#commit_plan)、[rebuild_committed_surface](src/context_pipeline/compact.rs#rebuild_committed_surface)、[SessionMutation](src/session/data/command.rs#SessionMutation)、[run](src/agent/core.rs#run)、[SessionController.start_manual_compact](src/client_protocol/controller/session_lifecycle.rs#SessionController.start_manual_compact)、[session_seq_g2_pipeline_reloads_fold_not_summary_plus_kept](src/types/death_list_gate.rs#session_seq_g2_pipeline_reloads_fold_not_summary_plus_kept)。

上层：[context-view](knowledge/contracts/context/context-view.md)。
契约展开：[compact-pipeline](knowledge/contracts/session/compact-pipeline.md)。
相关：[session-model](knowledge/contracts/session/session-model.md)。

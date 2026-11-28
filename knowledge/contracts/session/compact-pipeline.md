```node
node : compact-pipeline
status : enabled
summary : 压缩只改模型看到的历史，不改库里的事实；一次一个，写完读回才算成功。
x : 5364
y : 720
domain : harness-core
```

**目标：** 压缩只改模型看到的历史，不改库里的事实——该压的时候把已封口的旧历史折成「摘要 + 保留段」，写进库并读回校验，才算压成功。

**必须做到：**

- 一个会话同一时刻只有一个压缩任务：手动压缩遇到在跑的直接拒绝，自动和超限恢复等当前结果，不另开。
- 压缩在后台跑，对话可以继续；只有接近发送上限才等它。
- 普通压缩只看已封口、不会再变的历史；新消息和尾部不参与「要不要压、能不能压」，但仍算进发送上限。
- 模型报上下文超限时，摘要输入换成完整历史加当前新消息；放不下就从最老的丢，不拆工具调用/结果对，至少留最新用户消息；提交的压缩点仍只覆盖已封口历史。
- 完整成功 = 结果写进库 + 读回校验通过。写进去没校验不算成功；结果不直接交给对话，下一步对话自己从库里读。
- 读不回来先少量重试；仍读不回来就标「未验证」，让对话自己读一次——读成功且没超上限就继续，读失败、读到坏结果或仍超上限才算真失败。
- 失败只释放压缩任务，会话不算坏，下一次再试；同一轮不重复开。
- 回退、截断、删除之前先取消压缩并等它到终态——等的是「不会再提交」，不是等摘要跑完；这段时间也不许新的压缩进场。
- 结束只有完整成功、失败、取消三种，而且只有一个。

**绝对不做：**

- 不再机械裁剪历史——只有 keep-recent + 摘要一条路。
- 不碰发送线：0.8 是准备线，2% 是提前等待量，发送线取模型窗口和输入上限里更小的那个。
- 不改写、不删库里的旧消息：压缩点靠追加一行产生，旧行原样留着。
- 没有已封口的旧历史时不硬压；新消息本身太长直接拒绝。
- 把「已经进入提交的取消」当成「一定没写」——它可能已经提交。
- 把压缩结果直接塞给下一轮对话，绕开库。

依据：[try_begin_compaction_episode](src/session/manager.rs#try_begin_compaction_episode)（唯一压缩任务）、[CompactionEpisodeLease](src/session/manager.rs#CompactionEpisodeLease)（三种终态，Drop 兜底）、[CompactionInput](src/context_pipeline/compact.rs#CompactionInput)（sealed 边界）、[run_episode](src/context_pipeline/compact.rs#run_episode)（一次压缩、提交、读回）、[rebuild_committed_surface](src/context_pipeline/compact.rs#rebuild_committed_surface)（读回重试）、[readback_failure_action](src/context_pipeline/compact.rs#readback_failure_action)（读回失败分流）、[build_compaction_input](src/context_pipeline/compact.rs#build_compaction_input)（超限时从最老丢、只在用户消息处切）、[hard_limit](src/context_pipeline/context_meter.rs#hard_limit)、[compact_wait_margin](src/context_pipeline/limits.rs#compact_wait_margin)（发送线与等待线）、[compact_if_needed](src/runtime/exec.rs#compact_if_needed)（等待线、同轮不再开）、[call_model](src/runtime/exec.rs#call_model)（未验证读回的处理）、[try_begin_revert](src/session/manager.rs#try_begin_revert)、[remove_session_unchecked](src/session/manager.rs#remove_session_unchecked)（回退/删除取消并等收尾）。

上层：[context-compact](knowledge/contracts/context/context-compact.md)。相关：[session-write-path](knowledge/contracts/session/session-write-path.md)、[turn-reservation](knowledge/contracts/session/turn-reservation.md)。

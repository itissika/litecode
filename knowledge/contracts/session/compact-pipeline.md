```node
node : compact-pipeline
status : enabled
summary : 压缩是 session 自有的单一异步 episode；turn 只在硬线等待；机械与摘要二选一，只提交一个结果。
x : 5364
y : 720
domain : harness-core
```

**主张：** 压缩不是 turn 的同步步骤，也不是多条后台路径。它是 session 自有的单一 episode；所有触发走同一个入口，机械裁剪和摘要只是同一个决定的两个互斥结果。

## 守什么

1. **一个 session 一个 episode。** 同一时刻只有一条 compact 管线在跑；manual 撞上时必须拒绝或禁用，不 join。
2. **turn 和 compact 是两个状态轴。** turn 有 Idle / Running（可带 waiting_for_compact）；compact 有 Idle / Deciding / 提交阶段。`session busy` 是两者派生，不能把两者压成一个 running/idle。
3. **turn 只在硬线等待。** 低于硬线，turn 继续用旧视图，compact 在后台跑；到硬线才进入等待。
4. **成功必须提交，且只提交一种。** 机械替换（ReplaceSurface）或摘要 checkpoint（Compact）；没有 no-op 成功。
5. **机械与摘要互斥。** prune 先在内存里评估；只有单独把 meter 降到 0.8 以下才提交，否则不写，直接在原始 surface 上做摘要。不能 prune 提交完再 summary。
6. **前缀稳定，尾部可动。** compact 摘要的前缀只包含已封口行；正常 append/seal 只动 tail。提交时按当前 rows 重锚，允许尾部新增，不用全局 revision 拒绝。
7. **硬线是 no-send。** 到硬线等待当前 episode：成功继续；失败或取消则这次 turn 不能发送。低于硬线失败，turn 可用旧视图继续。
8. **失败释放 episode。** 下一次再撞硬线会开新 episode 重试；旧 episode 还在跑则等待，不重开。失败不是 session 永久停。
9. **revert / truncate / delete 必须 cancel + wait。** compact 不能在回退或删除后提交。
10. **终态只有 Succeeded / Failed / Cancelled。** Succeeded 必须提交一种结果；Failed/Cancelled 不提交；owner 收尾后才回 Idle。
11. **UI / telemetry 只读同一份 session 状态。** 不另建 compact 状态。

## 一次压缩怎么走

1. 触发方调用唯一入口；已有 episode 时，manual 拒绝，auto/overflow/next-turn 等待或消费当前结果，不另开。
2. 量当前上下文。
3. 在内存里构造 prune 候选，检查是否值得裁、是否单独能把 meter 降到 0.8 以下。
4. 能：提交机械替换，发成功事件，结束，不生成 compact point。
5. 不能：丢弃候选，不写 surface，在原始 rows 上构建摘要 plan。
6. 摘要成功后提交 checkpoint；失败或取消不提交。
7. 提交时重新读当前 rows，把摘要期间新增的 tail 放进 kept/tail。

机械替换会生成：替换事件、新 seq、新 surface、revision/receipt、成功事件。它不生成 summary checkpoint、summary 正文或 compact point；旧行仍在日志里。

## 硬线、失败、取消

- 0.8 是准备线，硬线是发送线。
- 硬线等待时失败或取消：turn 不能发送，清理后回 Idle；没有单独的 Ending 生命周期。
- 低于硬线失败：compact 回 Idle，turn 继续；下一次硬线再开新 episode。
- 超时是失败原因之一；具体秒数属于实现参数。

## overflow

模型从大窗口切到小窗口，旧 plan 失效，按新模型硬线重算。

- 裁的是发给 summarizer 的摘要请求输入，不是只裁最终上下文。
- 从最远历史开始丢，把摘要输入压到新模型硬线的 0.8 以下；这就是更激进，目的是让新模型接得住摘要请求。
- 裁剪点不能拆 tool 调用/结果对，至少保留最新 user 消息。
- 被丢掉的更早历史不进 summary；checkpoint 覆盖旧前缀，summary 文本明确标注“更早历史已省略”，旧行仍保留。
- 最终上下文是 summary + 保留的最新段；如果最新段本身超过硬线，失败并提示切回大模型、回退或新会话。

## 为什么关键

没有这条契约，同一 session 会出现多个改前缀的维护动作：下一轮在旧后台分支还在飞时 prune，manual 在后台分支在飞时进来，overflow 再开自己的恢复。结果不是磁盘坏，而是旧摘要盖新前缀、重复压缩、UI 状态被覆盖、正常 append 被全局 revision 误杀。

父节点：[@ key="context-compact"]。
相关：[@ key="session-write-path"]、[@ key="turn-reservation"]。

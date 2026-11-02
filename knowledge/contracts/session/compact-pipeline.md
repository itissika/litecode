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
4. **成功必须提交并交付，且只提交一种。** 机械替换（ReplaceSurface）或摘要 checkpoint（Compact），并通过读回校验成为可用 surface；没有 no-op 成功。
5. **机械与摘要互斥。** prune 先在内存里评估；只有单独把 meter 降到 0.8 以下才提交，否则不写，直接在原始 surface 上做摘要。不能 prune 提交完再 summary。
6. **普通输入封口，overflow 看全量。** 普通/manual 的 compact 输入只包含已封口行；overflow 一开始拿完整 surface + tail，但 checkpoint 仍只落在 sealed 行。正常 append/seal 只动 tail。提交后独立从 DB 重建 surface，尾部自然留在 checkpoint 之后，不用全局 revision 拒绝。
7. **硬线是 no-send。** 到硬线等待当前 episode：成功继续；失败后由 session 读一次当前 surface，读成功且不超硬线才继续，读失败或已知坏 surface 才停；取消停。低于硬线失败，turn 继续，由这次 session 读决定。
8. **失败释放 episode。** 下一次再撞硬线会开新 episode 重试；旧 episode 还在跑则等待，不重开。失败不是 session 永久停。
9. **revert / truncate / delete 必须 cancel + wait。** compact 不能在回退或删除后提交。
10. **终态只有 Succeeded / Failed / Cancelled。** Succeeded 必须是“已提交且已交付可用 surface”；Failed 可发生在 commit 前（旧 surface 继续）或 commit 后读回未验成（不是成功，交给 session 再读；读失败才停）；读回是坏 surface 才直接停；manual 没有 turn，只报错；Cancelled 不提交；owner 收尾后才回 Idle。
11. **UI / telemetry 只读同一份 session 状态。** 不另建 compact 状态。

## 一次压缩怎么走

1. 触发方调用唯一入口；已有 episode 时，manual 拒绝，auto/overflow/next-turn 等待或消费当前结果，不另开。
2. 量当前上下文。
3. 在内存里构造 prune 候选，检查是否值得裁、是否单独能把 meter 降到 0.8 以下。
4. 能：提交机械替换，发成功事件，结束，不生成 compact point。
5. 不能：丢弃候选，不写 surface，在原始 rows 上构建摘要 plan。
6. 摘要成功后提交 checkpoint；失败或取消不提交。
7. 提交与交付分开：写库成功即提交事实；提交成功后独立读回并校验 durable surface。读回失败先做少量重试；仍读不回来则标记 unverified，交给 session 自己读，不能报 Succeeded。读回来但校验发现是坏 surface 直接停；manual 只报错。保留 commit 事实、不回滚 DB，也不合成 surface。

机械替换会生成：替换事件、新 seq、新 surface、revision/receipt、成功事件。它不生成 summary checkpoint、summary 正文或 compact point；旧行仍在日志里。

## 硬线、失败、取消

- 0.8 是准备线，硬线是发送线。
- 硬线等待时失败或取消：turn 不能发送，清理后回 Idle；没有单独的 Ending 生命周期。
- 低于硬线失败：compact 回 Idle，turn 继续；下一次硬线再开新 episode。
- 超时是失败原因之一；具体秒数属于实现参数。

## overflow

模型切换不主动 cancel 正在跑的旧 plan：旧 episode 可自然提交；新 turn 用新模型重新测量，仍超再开新 episode。

- 输入是完整 surface + tail；裁的是发给 summarizer 的摘要请求输入，不是只裁最终上下文。
- 从最远历史开始丢，把摘要输入压到新模型硬线的 0.8 以下；这就是更激进，目的是让新模型接得住摘要请求。
- 裁剪点不能拆 tool 调用/结果对，至少保留最新 user 消息。
- 被丢掉的更早历史不进 summary；checkpoint 覆盖旧前缀，summary 文本明确标注“更早历史已省略”，旧行仍保留。
- 最终上下文是 summary + 保留的最新段；如果最新段本身超过硬线，失败并提示切回大模型、回退或新会话。

## 为什么关键

没有这条契约，同一 session 会出现多个改前缀的维护动作：下一轮在旧后台分支还在飞时 prune，manual 在后台分支在飞时进来，overflow 再开自己的恢复。结果不是磁盘坏，而是旧摘要盖新前缀、重复压缩、UI 状态被覆盖、正常 append 被全局 revision 误杀。

## 落地要点

- **manual 完全独立**：不占 turn activity，可在 turn 运行中启动；turn 可继续追加/封口尾部；只有已有 episode 时拒绝。manual 不阻塞发送。
- **唯一 episode**：auto/manual/overflow 都经 `try_begin_compaction_episode`；`Abandoned` 删除，终态只有 Succeeded / Failed / Cancelled。
- **硬线**：`min(产品窗口, 物理 prompt 上限)` 是 no-send 线；2% margin 只是提前等待点，不是硬线。
- **硬线等待失败不重试**：本轮结束，下一轮再开；低于硬线失败用旧视图继续。
- **已封口前缀**：摘要/机械裁剪只碰 `LogState::Final` 行；第一行 in-progress 及其后留在 kept/tail。
- **提交**：`Compact` / `ReplaceSurface` 不带全局 expected_revision；用 `prior_checkpoint + kept_from + from` 结构守卫，revision 从事务内当前值 +1，operation id 幂等。
- **摘要输入**：从最老整段对话丢到 `0.8 × (新模型物理 prompt 上限 − 摘要输出预留)`；可丢开头 checkpoint；丢过则 summary 标注“更早历史已省略”。
- **输入分开**：普通/manual 的 prune/summary 只读 sealed prefix；manual 准入与 auto 0.8 触发也只测 sealed prefix；tail 不参与是否压缩、能否压缩、算不算成功。完整 working set 只用于真实发送的 hard/wait 门。overflow 是同一管线的全量输入：完整 surface + tail，摘要输入从最老历史丢到新模型物理线 0.8，checkpoint 仍只覆盖 sealed 行，最终 meter 仍超硬线才失败。
- **分支封口**：`select_method(sealed_prefix)` 先选 Prune 或 Summary，选定后失败不切换；prune 选择不写 surface。
- **提交/交付分开**：分支只产出提交计划；公共 stage 只提交一次，写库成功是提交事实；读回并校验出可用的 durable surface 才是成功。瞬时读失败做有限重试；仍读不回来是 unverified，交给 session 自取，session 读失败才停；校验出坏 surface 直接停，manual 只报错。不回滚 DB，不合成 surface，也不另加 surface 交接层。
- **有限重试**：压缩的读/提交重试有固定次数，不在 busy store 上无限自旋；session 每次取 surface 的读取失败必须向 turn 报错，不能用旧 fold 继续。
- **终态**：每个结束路径（成功、失败、取消、预检查失败、线程起不来、异常退出）都先收尾再发唯一终态；session 只看到这一份结果。

父节点：[@ key="context-compact"]。
相关：[@ key="session-write-path"]、[@ key="turn-reservation"]。
